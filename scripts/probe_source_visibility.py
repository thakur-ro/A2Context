"""Measure source visibility on a recorded Qwen2 answer or tool proposal."""

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


def mark_text_source(runtime, messages, tools, anchor, text, marker):
    pointer = re.fullmatch(r"/messages/(0|[1-9][0-9]*)/content", anchor["pointer"])
    if pointer is None:
        return None
    index = int(pointer[1])
    if index >= len(messages) or not isinstance(messages[index].get("content"), str):
        raise ValueError("A request fragment points outside the recorded text messages.")
    content = messages[index]["content"].encode("utf-8")
    start = anchor["text_range"]["start_byte"]
    end = anchor["text_range"]["end_byte"]
    if not 0 <= start < end <= len(content) or content[start:end] != text.encode("utf-8"):
        raise ValueError("A captured source differs from its recorded request fragment.")

    tagged = list(messages)
    tagged[index] = {**messages[index], "content": content[:start].decode() + marker + content[end:].decode()}
    return runtime.render_prompt(tagged, tools)


def aligned_source(runtime, prompt, tokens, offsets, messages, tools, item):
    text = item.get("text")
    if item["availability"] != "included" or text is None:
        return {"unavailable": "text_not_captured"}

    # Mark the exact occurrence before rendering so repeated text maps by location.
    marker = "NEXUSWEAVE_SOURCE_MARKER"
    while marker in prompt:
        marker += "_"
    source = {}
    anchor = item.get("request_fragment")
    if anchor is not None:
        rendered = mark_text_source(runtime, messages, tools, anchor, text, marker)
        if rendered is None:
            return {"unavailable": "unsupported_request_field"}
    elif item["kind"] == "tool_definition" and tools:
        try:
            canonical = json.dumps(json.loads(text), sort_keys=True, allow_nan=False)
        except ValueError:
            return {"unavailable": "request_object_not_found"}
        matches = [
            i for i, tool in enumerate(tools)
            if isinstance(tool, dict) and json.dumps(tool, sort_keys=True, allow_nan=False) == canonical
        ]
        if not matches:
            return {"unavailable": "request_object_not_found"}
        if len(matches) != 1:
            return {"unavailable": "request_object_ambiguous"}
        index = matches[0]
        tagged = list(tools)
        tagged[index] = {marker: None}
        marker = json.dumps(tagged[index], ensure_ascii=False)
        text = json.dumps(tools[index], ensure_ascii=False)
        rendered = runtime.render_prompt(messages, tagged)
        source["request_pointer"] = f"/tools/{index}"
    else:
        return {"unavailable": "no_request_fragment"}

    if rendered.count(marker) != 1 or rendered.replace(marker, text) != prompt:
        return {"unavailable": "rendered_source_unaligned"}
    left = rendered.index(marker)
    right = left + len(text)
    overlapping = [(i, a, b) for i, (a, b) in enumerate(offsets) if a < right and left < b and a < b]
    if not overlapping:
        return {"unavailable": "token_boundary_unaligned"}
    first, last = overlapping[0][0], overlapping[-1][0] + 1
    if len(overlapping) != last - first or last >= len(offsets):
        return {"unavailable": "token_boundary_unaligned"}
    start, end = min(a for _, a, _ in overlapping), max(b for _, _, b in overlapping)
    if start > left or end < right:
        return {"unavailable": "token_boundary_unaligned"}
    before, after = prompt[start:left], prompt[right:end]
    decoded = runtime.tokenizer.decode(tokens[first:last], skip_special_tokens=False,
                                       clean_up_tokenization_spaces=False)
    if (before.strip(" \t\r\n") or after.strip(" \t\r\n")
            or decoded != before + text + after):
        return {"unavailable": "token_boundary_unaligned"}
    if before or after:
        source["boundary_whitespace"] = {"before": before, "after": after}
    source["token_range"] = [first, last]
    return source


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
    tools = request.get("tools")
    prompt, tokens, offsets, _ = runtime.prompt_input(messages, tools)
    answer, generated = runtime.generate(prompt, tokens, request)
    proposal = None
    if decision["kind"] == "tool_call" and tools:
        proposal, _ = runtime.parse_proposal(answer, tools)
        recorded = {"name": decision["label"], "arguments": decision["outcome"]}
        reproduced = json.dumps(proposal, sort_keys=True, allow_nan=False) == json.dumps(recorded, sort_keys=True, allow_nan=False)
    else:
        reproduced = decision["kind"] == "model_output" and answer == decision["outcome"]
    if not generated or not reproduced:
        raise ValueError("The model must reproduce the recorded text or tool decision.")
    readouts = [(len(tokens) + i - 1, token, None) for i, token in enumerate(generated)]
    clean = tokens + generated
    baseline = runtime.scores(clean, readouts, {})
    no_op = masked_scores(runtime, clean, readouts)
    error = max(abs(a - b) for a, b in zip(baseline, no_op, strict=True))
    if not math.isfinite(error) or error > 1e-5:
        raise ValueError("The all-visible control did not reproduce the unmasked token scores.")
    sources = []
    for item in call["context"]:
        source = {"context_id": item["id"], **aligned_source(runtime, prompt, tokens, offsets, messages, tools, item)}
        if "token_range" in source:
            source.update(
                captured_text_sha256=sha256(item["text"].encode()).hexdigest(),
                masked_log_probs=masked_scores(runtime, clean, readouts, [source["token_range"]]),
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
    if proposal is not None:
        report["baseline_tool_call"] = proposal
    boundaries = {source["context_id"]: source["boundary_whitespace"]
                  for source in sources if "boundary_whitespace" in source}
    if boundaries:
        digest = sha256(json.dumps(boundaries, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        report["id"] += "-whitespace-" + digest[:16]
    plan = pair_plan(sources, pairs, pair_duplicates)
    if plan:
        report["pairs"] = [
            {"context_ids": ids, "masked_log_probs": masked_scores(runtime, clean, readouts, ranges)}
            for ids, ranges in plan
        ]
        digest = sha256(json.dumps([ids for ids, _ in plan], separators=(",", ":")).encode()).hexdigest()
        report["id"] += "-" + digest[:16]
    return report


def output_targets(specs, token_count):
    targets = {}
    for key, start, end in specs:
        start, end = int(start), int(end)
        if not key or key.strip() != key or key in targets or not 0 <= start < end <= token_count:
            raise ValueError("Output targets need unique keys and nonempty ranges in the recorded output.")
        targets[key] = [start, end]
    return [{"key": key, "token_range": token_range} for key, token_range in targets.items()]


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
    parser.add_argument("--target", action="append", nargs=3, default=[], metavar=("KEY", "START", "END"),
                        help="Name an output token range by zero-based start and exclusive end.")
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
    if args.target:
        try:
            report["targets"] = output_targets(args.target, len(report["output_token_ids"]))
        except ValueError as error:
            parser.error(str(error))
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
