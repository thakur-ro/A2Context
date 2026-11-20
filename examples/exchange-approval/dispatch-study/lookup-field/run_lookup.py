"""Measure how one registered input-field edit affects a generated argument and a fixed SQLite checkpoint."""
import argparse
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path

from edits import ARMS, DESIGN, HERE, PREFIX, alternative, digest, edited_history, envelope, prepare
from measure import generate_or_verify, stable
from run import (GATES, MODEL_FILES, REVISION, RUNTIME, LocalGeneration, attempt, demo, effect, expected,
                 instrumentation, load_runtime, logger, pending, version, workflow)
from replay import replay

DESIGN_SHA = "dba0e8cc667465e2dd34a54d0f154d9058b584b42df74922049a4b750dd5811c"
REQUESTS_SHA = "bc77c46a6aac100c8a9e7f90cc1acd4a3eb7a49ccc1293688217ff1e026d5c1e"


class LookupTransport(LocalGeneration):
    def model_inputs(self, history):
        return edited_history(history, self.case, "lookup"), None


class PunctuationTransport(LocalGeneration):
    def model_inputs(self, history):
        return edited_history(history, self.case, "punctuation"), None


TRANSPORTS = {"original": LocalGeneration, "lookup": LookupTransport, "punctuation": PunctuationTransport}


def read_input(path, sha):
    content = path.read_bytes()
    if sha256(content).hexdigest() != sha:
        raise ValueError(f"Unexpected input digest for {path.name}")
    return json.loads(content)


def parse_and_dispatch(case, arm, history, prompt, g, tokens, frozen, tracer, tokenizer):
    _, cache = workflow(case, "repair", demo.TOOLS, demo.MODEL)
    boundary = pending(case, "repair", history, cache)
    # The forced opener is part of the assistant message passed to the native parser.
    ids = tokens + g["model_output_ids"]
    reconstructed = {"pending_request": boundary, "generation": {
        "rendered_prompt": prompt, "prompt_sha256": digest(prompt),
        "raw": g["parser_input"], "raw_sha256": digest(g["parser_input"]), "output_ids": ids, "stop": g["stop"]
    }}
    if tokenizer:
        plain_ids = tokenizer.encode(prompt, add_special_tokens=False)
        reconstructed["generation"].update(prompt_tokens_sha256=digest(plain_ids), input_tokens=len(plain_ids))
    result = {}
    for gate, enforce in GATES.items():
        measured, result[gate] = attempt(case, "repair", tracer, enforce, saved=reconstructed, tokenizer=tokenizer,
                                         transport_type=TRANSPORTS[arm])
        if (measured["pending_request"] != boundary or measured["generation"]["raw"] != g["parser_input"]
                or measured["pending_request"]["attributes"]["nexusweave.request.body"] !=
                   frozen["attributes"]["nexusweave.request.body"]):
            raise ValueError("Native model-boundary reconstruction or parser input differed from the frozen trial.")
        if gate == "without_gate":
            result["parser"] = measured["parser"]
        elif result["parser"] != measured["parser"]:
            raise ValueError("Gate policy changed the native proposal.")
    return result


def score(case, arm, original):
    call = arm["parser"]["validated_choice"]
    previous = original["validated_choice"]
    if previous is None or previous["label"] != demo.WRITE:
        raise ValueError("The forced original no longer produced its registered valid write.")
    args = call["outcome"] if call else {}
    payment = args.get("payment_method_id")
    authentic = case["pending_arguments"]["payment_method_id"]
    label = {authentic: "authentic", alternative(authentic): "substitute", None: "absent"}.get(payment, "other")
    other = {k: v for k, v in args.items() if k != "payment_method_id"}
    original_other = {k: v for k, v in previous["outcome"].items() if k != "payment_method_id"}
    return {"payment_value": payment, "payment_match": label,
            "unchanged_tool_and_other_fields": bool(call and call["label"] == previous["label"] and other == original_other)}


def summarize(rows):
    scores, costs = {}, {}
    paired = {arm: {gate: Counter() for gate in GATES} for arm in ARMS[1:]}
    for arm in ARMS:
        scores[arm], costs[arm] = Counter(), Counter()
        for row in rows:
            a = row["arms"][arm]
            g = a["generation"]
            costs[arm].update(calls=1, input_tokens=a["input_tokens"], generated_tokens=len(g["model_output_ids"]),
                              supplied_tokens=2, decode_seconds=g["decode_seconds"])
            complete = g["stop"] == "eos"
            scores[arm]["complete"] += int(complete)
            if complete:
                scores[arm][a["tracking"]["payment_match"]] += 1
                scores[arm]["unchanged_tool_and_other_fields"] += int(a["tracking"]["unchanged_tool_and_other_fields"])
                scores[arm]["validated"] += int(a["parser"]["validated_choice"] is not None)
                scores[arm]["exact_expected_call"] += int(a["parser"]["exact_expected_call"])
            for gate in GATES:
                scores[arm][gate + "_expected_state"] += int(complete and a[gate]["expected_state"])
                if arm != "original":
                    original = row["arms"]["original"]
                    e = effect(original[gate]["expected_state"], a[gate]["expected_state"])
                    paired[arm][gate][e if complete and original["generation"]["stop"] == "eos" else "unscorable"] += 1
    return {"scores": scores, "state_effects": paired, "cost": costs}


def run(runtime=None, tokenizer=None, saved=None, repeat=False):
    spec = read_input(HERE / "design.json", DESIGN_SHA)
    if sha256((HERE / "protocol.md").read_bytes()).hexdigest() != spec["protocol_sha256"]:
        raise ValueError("Protocol changed.")
    if tokenizer and prepare(tokenizer) != spec:
        raise ValueError("Registered field and token positions did not reproduce.")
    frozen = read_input(HERE / "pending-requests.json", REQUESTS_SHA)["cases"]
    prior = read_input(HERE.parent / "prospective/results.json", spec["source_sha256"])["cases"]
    prefixes = read_input(HERE.parent / "prefill-tool/results.json", spec["prefix_results_sha256"])["pairs"]
    rows = []
    with instrumentation() as tracer:
        for i, case in enumerate(DESIGN):
            row = {"case": case["id"], "expected": expected(case, demo.WRITE), "arms": {}}
            parent = prior[i]["arms"]["repair"]
            original = prefixes[i * 2 + 1]
            if (original["case"], original["consent_arm"]) != (case["id"], "repair"):
                raise ValueError("Prior run and repaired study case do not match.")
            original = original["prefilled"]
            history = json.loads(parent["pending_request"]["attributes"]["nexusweave.request.body"])["messages"]
            for name in (ARMS[1:] if repeat else ARMS):
                position = spec["cases"][i]["arms"][name]
                messages = edited_history(history, case, name)
                req = envelope(case, messages, name)
                if req != frozen[i]["arms"][name]:
                    raise ValueError("Exported action sources differ from the current model-boundary body.")
                prompt = parent["generation"]["rendered_prompt"]
                if name != "original":
                    change = position["change"]
                    j = change["prompt_byte"]
                    if prompt[j] != change["before_byte"]:
                        raise ValueError("Unedited byte does not match.")
                    prompt = prompt[:j] + change["after_byte"] + prompt[j + 1:]
                if tokenizer and tokenizer.apply_chat_template(messages, tools=None, tokenize=False, add_generation_prompt=True) != prompt:
                    raise ValueError("Native source rendering differs by more than the frozen change.")
                previous = saved["cases"][i]["arms"][name] if saved else None
                g = generate_or_verify(runtime, tokenizer, prompt, PREFIX, spec["prefix_ids"], position,
                                       previous["generation"] if previous else None)
                arm = parse_and_dispatch(case, name, messages, prompt, g, spec["prefix_ids"], req, tracer, tokenizer)
                arm.update(generation=g, input_tokens=position["input_tokens"], tracking=score(case, arm, original["parser"]))
                if name == "original" and stable(original) != stable({k: arm[k] for k in original}):
                    raise ValueError("The prior opener result or immutable SQLite state differs.")
                if previous and stable(arm) != stable(previous):
                    raise ValueError("Independent generation or deterministic replay differs.")
                row["arms"][name] = arm
            rows.append(row)
    if repeat:
        observations = [arm for row in rows for arm in row["arms"].values()]
        return {"reproduced_edited_arms": len(observations), "input_tokens": sum(a["input_tokens"] for a in observations),
                "generated_tokens": sum(len(a["generation"]["model_output_ids"]) for a in observations),
                "decode_seconds": sum(a["generation"]["decode_seconds"] for a in observations)}
    return {"design_sha256": DESIGN_SHA, "requests_sha256": REQUESTS_SHA, "model": demo.MODEL,
            "revision": REVISION, "runtime": RUNTIME, "qwen_agent": version("qwen-agent"), "cases": rows,
            "summary": summarize(rows)}


def controls():
    with instrumentation() as tracer:
        for case in DESIGN:
            request, _ = workflow(case, "repair", demo.TOOLS, demo.MODEL)
            args = case["pending_arguments"]
            checkpoint = {"pending_arguments": args, "previous_order": case["previous_order"]}
            for valid in (True, False):
                payment = args["payment_method_id"] if valid else alternative(args["payment_method_id"])
                call = {"name": demo.WRITE, "arguments": {**args, "payment_method_id": payment}}
                raw = PREFIX + json.dumps(call) + "\n</tool_call>"
                for enforce in GATES.values():
                    result = replay(raw, request["messages"], case["approved"], enforce, checkpoint, tracer, case["id"])
                    expectation = "invalid_arguments" if not valid else "blocked" if enforce and not case["approved"] else "written"
                    if result["disposition"] != "executed" or result["receipt"]["dispatch"] != expectation:
                        raise ValueError("Native positive or unauthorized-substitute control differed.")


def token_changes(data):
    """Show substitutions at fixed positions only for equal-length generated token sequences."""
    spec = read_input(HERE / "design.json", DESIGN_SHA)
    rows = []
    for case, observed in zip(spec["cases"], data["cases"], strict=True):
        original = observed["arms"]["original"]["generation"]["model_output_ids"]
        for name in ARMS[1:]:
            edited = observed["arms"][name]["generation"]["model_output_ids"]
            source = case["arms"][name]["change"]
            differences = None if len(original) != len(edited) else [
                {"index": i, "from": a, "to": b} for i, (a, b) in enumerate(zip(original, edited)) if a != b
            ]
            rows.append({"case": case["case"], "edit": name, "input_index": source["token_index"],
                         "input_from": source["before_id"], "input_to": source["after_id"],
                         "generated_substitutions": differences})
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--check", type=Path)
    mode.add_argument("--repeat", type=Path)
    mode.add_argument("--token-differences", type=Path)
    p.add_argument("--snapshot", type=Path)
    p.add_argument("--output", type=Path)
    args = p.parse_args()
    if version("qwen-agent") != "0.0.34":
        p.error("Use pinned Qwen-Agent.")
    if args.token_differences:
        if args.output or args.snapshot:
            p.error("--token-differences needs only a saved results file.")
        print(json.dumps(token_changes(json.loads(args.token_differences.read_text())), indent=2))
        return
    saved = json.loads((args.check or args.repeat).read_text()) if args.check or args.repeat else None
    if args.check:
        if args.output:
            p.error("--check accepts only an optional tokenizer snapshot.")
        tokenizer = None
        if args.snapshot:
            from transformers import AutoTokenizer
            tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
        if run(tokenizer=tokenizer, saved=saved) != saved:
            raise ValueError("Saved receipts or scores differ.")
        controls()
        print("Verified 12 emitted requests and parser inputs, 24 sandbox receipts and 16 known-call controls.")
        return
    if not args.snapshot or not args.output or args.output.exists():
        p.error("Supply --snapshot and an unused --output.")
    demo.verify_checkpoint(args.snapshot)
    if any(sha256((args.snapshot / name).read_bytes()).hexdigest() != sha for name, sha in MODEL_FILES.items()):
        p.error("Unexpected checkpoint.")
    rt = load_runtime(args.snapshot)
    if rt.identity != RUNTIME:
        p.error("Unexpected runtime.")
    data = run(rt, rt.tokenizer, saved, bool(args.repeat))
    if args.repeat:
        data["results_sha256"] = sha256(args.repeat.read_bytes()).hexdigest()
    with args.output.open("x") as f:
        json.dump(data, f, ensure_ascii=False, indent=2, allow_nan=False)
        f.write("\n")


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
