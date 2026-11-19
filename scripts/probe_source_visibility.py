"""Measure source visibility on a recorded Qwen2 answer without writing prompt edits."""

import argparse
from contextlib import ExitStack
from hashlib import sha256
from itertools import combinations
import json
import math
from pathlib import Path
import re

from activation_timeline import output_fragments
from probe_transformers_answer_timeline import load_runtime


def aligned_range(runtime, prompt, offsets, messages, item):
    if item["availability"] != "included" or item.get("text") is None:
        return None, "text_not_captured"
    anchor = item.get("request_fragment")
    if anchor is None:
        return None, "no_request_fragment"
    pointer = re.fullmatch(r"/messages/(0|[1-9][0-9]*)/content", anchor["pointer"])
    if pointer is None:
        return None, "unsupported_request_field"
    index = int(pointer[1])
    if index >= len(messages) or not isinstance(messages[index].get("content"), str):
        raise ValueError("A request fragment points outside the recorded text messages.")
    content = messages[index]["content"].encode("utf-8")
    start = anchor["text_range"]["start_byte"]
    end = anchor["text_range"]["end_byte"]
    text = item["text"]
    if not 0 <= start < end <= len(content) or content[start:end] != text.encode("utf-8"):
        raise ValueError("A captured source differs from its recorded request fragment.")

    # Mark the anchored occurrence before rendering so repeated text maps by location.
    marker = "NEXUSWEAVE_SOURCE_MARKER"
    while marker in prompt:
        marker += "_"
    tagged = list(messages)
    tagged[index] = {**messages[index], "content": content[:start].decode() + marker + content[end:].decode()}
    rendered = runtime.tokenizer.apply_chat_template(tagged, tokenize=False, add_generation_prompt=True)
    if rendered.count(marker) != 1 or rendered.replace(marker, text) != prompt:
        return None, "rendered_source_unaligned"
    left = rendered.index(marker)
    right = left + len(text)
    overlapping = [(i, a, b) for i, (a, b) in enumerate(offsets) if a < right and left < b and a < b]
    if (not overlapping or any(a < left or b > right for _, a, b in overlapping)
            or overlapping[0][1] != left or overlapping[-1][2] != right):
        return None, "token_boundary_unaligned"
    first, last = overlapping[0][0], overlapping[-1][0] + 1
    if len(overlapping) != last - first or last >= len(offsets):
        return None, "token_boundary_unaligned"
    return [first, last], None


def masked_scores(runtime, tokens, readouts, ranges=()):
    torch = runtime.torch
    attention = torch.ones((1, len(tokens)), dtype=torch.long)
    positions = torch.arange(len(tokens)).unsqueeze(0)
    with ExitStack() as hooks:
        if ranges:
            for start, end in ranges:
                attention[:, start:end] = 0

            def check_mask(_layer, _args, inputs):
                mask = inputs["attention_mask"]
                for start, end in ranges:
                    masked = mask[..., end:, start:end]
                    if masked.numel() == 0 or not torch.all(masked == torch.finfo(mask.dtype).min):
                        raise ValueError("A decoder block did not mask the requested source keys.")

            for layer in runtime.model.model.layers:
                hook = layer.register_forward_pre_hook(check_mask, with_kwargs=True)
                hooks.callback(hook.remove)
        return runtime.scores(tokens, readouts, {}, attention_mask=attention, position_ids=positions)


def disjoint(first, second):
    return first[1] <= second[0] or second[1] <= first[0]


def pair_plan(sources, requested, include_duplicates):
    measured = {source["context_id"]: source for source in sources if "token_range" in source}
    plan = set()
    for first, second in requested:
        if first == second or first not in measured or second not in measured:
            raise ValueError(f"A pair needs two different measured sources: {first}, {second}.")
        if not disjoint(measured[first]["token_range"], measured[second]["token_range"]):
            raise ValueError(f"A pair needs non-overlapping prompt tokens: {first}, {second}.")
        plan.add(tuple(sorted((first, second))))
    if include_duplicates:
        by_text = {}
        for source in measured.values():
            by_text.setdefault(source["captured_text_sha256"], []).append(source)
        for matches in by_text.values():
            for first, second in combinations(matches, 2):
                if disjoint(first["token_range"], second["token_range"]):
                    plan.add(tuple(sorted((first["context_id"], second["context_id"]))))
    return [(ids, [measured[key]["token_range"] for key in ids]) for ids in sorted(plan)]


def measure(runtime, model_path, model_id, revision, location, call, decision, *, pairs=(), pair_duplicates=False):
    request = json.loads(call["recorded_request"]["body"])
    messages = request["messages"]
    prompt, tokens, offsets, _ = runtime.prompt_input(messages, request.get("tools"))
    answer, generated = runtime.generate(prompt, tokens, request)
    if decision["kind"] != "model_output" or not generated or answer != decision["outcome"]:
        raise ValueError("The model must reproduce the recorded text decision.")
    readouts = [(len(tokens) + i - 1, token, None) for i, token in enumerate(generated)]
    clean = tokens + generated
    baseline = runtime.scores(clean, readouts, {})
    no_op = masked_scores(runtime, clean, readouts)
    error = max(abs(a - b) for a, b in zip(baseline, no_op, strict=True))
    if not math.isfinite(error) or error > 1e-5:
        raise ValueError("The all-visible control did not reproduce the unmasked token scores.")
    sources = []
    for item in call["context"]:
        source = {"context_id": item["id"]}
        token_range, reason = aligned_range(runtime, prompt, offsets, messages, item)
        if token_range is None:
            source["unavailable"] = reason
        else:
            source.update(
                token_range=token_range,
                captured_text_sha256=sha256(item["text"].encode()).hexdigest(),
                masked_log_probs=masked_scores(runtime, clean, readouts, [token_range]),
            )
        sources.append(source)
    if not any("masked_log_probs" in source for source in sources):
        raise ValueError("No captured context source aligns to text in the rendered prompt.")
    report = {
        "version": 1, "kind": "source_visibility", "id": "qwen2-source-visibility",
        "location": location, "model_id": model_id, "model_revision": revision, "runtime": runtime.identity,
        "prompt_sha256": sha256(prompt.encode()).hexdigest(),
        "tokenizer_sha256": sha256((model_path / "tokenizer.json").read_bytes()).hexdigest(),
        "request_sha256": sha256(call["recorded_request"]["body"].encode()).hexdigest(),
        "prompt_token_count": len(tokens), "baseline_answer": answer, "output_token_ids": generated,
        "fragments": output_fragments(runtime.tokenizer, answer, generated),
        "baseline_log_probs": baseline, "all_visible_max_abs_error": error, "sources": sources,
    }
    plan = pair_plan(sources, pairs, pair_duplicates)
    if plan:
        report["pairs"] = [
            {"context_ids": ids, "masked_log_probs": masked_scores(runtime, clean, readouts, ranges)}
            for ids, ranges in plan
        ]
        digest = sha256(json.dumps([ids for ids, _ in plan], separators=(",", ":")).encode()).hexdigest()
        report["id"] += "-" + digest[:16]
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--call-id", required=True)
    parser.add_argument("--decision-id", required=True)
    parser.add_argument("--pair", action="append", nargs=2, default=[], metavar=("FIRST", "SECOND"),
                        help="Measure two context IDs together. Repeat to measure more pairs.")
    parser.add_argument("--pair-duplicates", action="store_true",
                        help="Also test pairs with identical captured text and separate token ranges.")
    args = parser.parse_args()
    runs = json.loads(args.evidence.read_text(encoding="utf-8"))
    run = next((run for run in runs if run["id"] == args.run_id), None)
    call = next((call for call in run["calls"] if call["id"] == args.call_id), None) if run else None
    decision = next((d for d in call["decisions"] if d["id"] == args.decision_id), None) if call else None
    if not decision or not call.get("recorded_request"):
        parser.error("The selected decision and captured request are required.")
    location = {"run_id": args.run_id, "call_id": args.call_id, "decision_id": args.decision_id}
    runtime = load_runtime(args.model)
    with runtime.torch.inference_mode():
        report = measure(runtime, args.model, args.model_id, args.revision, location, call, decision,
                         pairs=args.pair, pair_duplicates=args.pair_duplicates)
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
