"""Rank sources before generating the next airline proposal. Retain hashes instead of raw customer data."""
import argparse
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import re
import sys
from time import perf_counter

from inputs import HERE, canonical, cohort, digest

ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "examples" / "exchange-approval" / "dispatch-study"))
from probe_transformers_answer_timeline import TransformersRuntime, load_runtime
from score_qwen_prefill import first_step, js_bits
from study import REVISION, RUNTIME, demo, generate

SELECTION_SHA = "b82704c339aedfd48f034f36e160bcd55624dc71d10d6aab3577fc37b737d523"
EXTRA_MODEL_FILES = {
    "config.json": "18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45",
    "generation_config.json": "e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6",
    "tokenizer_config.json": "006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680",
}


def action(calls, kinds):
    return {"kind": "tools" if calls else "text",
            "calls": [{"name": name, "arguments_sha256": digest(canonical(args))} for name, args in calls],
            "write": any(kinds[name] == "WRITE" for name, _ in calls)}


def classify(raw, stop, tools, kinds):
    if stop != "eos":
        return {"kind": "truncated", "calls": [], "write": None}
    blocks = re.findall(r"<tool_call>.*?</tool_call>", raw, re.DOTALL)
    if raw.count("<tool_call>") != len(blocks) or raw.count("</tool_call>") != len(blocks):
        return {"kind": "invalid", "calls": [], "write": None}
    definitions = {tool["function"]["name"]: tool["function"]["parameters"] for tool in tools}
    types = {"integer": int, "string": str, "array": list}
    calls = []
    for block in blocks:
        try:
            parsed, _ = TransformersRuntime.parse_proposal(block, tools)
        except (TypeError, ValueError):
            return {"kind": "invalid", "calls": [], "write": None}
        if parsed is None:
            return {"kind": "invalid", "calls": [], "write": None}
        name, args = parsed["name"], parsed["arguments"]
        props = definitions[name]["properties"]
        if args.keys() != props.keys():
            return {"kind": "invalid", "calls": [], "write": None}
        for key, prop in props.items():
            value = args[key]
            if type(value) is not types[prop["type"]]:
                return {"kind": "invalid", "calls": [], "write": None}
            if isinstance(value, list) and any(not isinstance(item, dict) for item in value):
                return {"kind": "invalid", "calls": [], "write": None}
        calls.append((name, args))
    return action(calls, kinds)


def measure(runtime, arms, tools, kinds, audit):
    ids_by_arm, saved = {}, {}
    for name, msgs in arms.items():
        start = perf_counter()
        prompt, ids, _, _ = runtime.prompt_input(msgs, tools)
        ids_by_arm[name] = ids
        saved[name] = {"prompt_sha256": digest(prompt), "input_tokens": len(ids)}
        if name == "withdrawal":
            continue
        distribution = first_step(runtime, ids)
        if name == "original":
            baseline = distribution
        else:
            saved[name]["js_bits"] = js_bits(runtime.torch, baseline, distribution)
        saved[name]["probe_seconds"] = perf_counter() - start
    ranked = sorted((n for n in saved if "js_bits" in saved[n]), key=lambda n: -saved[n]["js_bits"])
    for name, ids in ids_by_arm.items():
        result = generate(runtime, ids)
        raw = result.pop("raw")
        saved[name].update(result, output_sha256=digest(raw), action=classify(raw, result["stop"], tools, kinds))
        if audit is not None:
            audit[name] = raw
    return ranked, saved


def summary(cases):
    selectors = ("ranked", "first", "latest_message", "latest_tool")
    counts = {
        "complete_cases": 0,
        "incomplete_cases": [],
        "withdrawal": Counter(),
        "withdrawal_from_valid_original_write": Counter(),
        "historical": Counter(),
        "cost": Counter(),
    }
    for metric in ("primary", "secondary"):
        counts[metric] = {name: {"changed": 0, "paired_scorable": 0, "paired_changed": 0} for name in selectors}

    for case in cases:
        arms = case["arms"]
        original = arms["original"]["action"]
        valid = {name: arm["action"]["write"] is not None for name, arm in arms.items()}
        complete = all(ok for name, ok in valid.items() if name != "withdrawal")
        if complete:
            counts["complete_cases"] += 1
        else:
            counts["incomplete_cases"].append(case["task"])

        choices = {
            "ranked": case["ranked"][0], "first": "policy",
            "latest_message": "user", "latest_tool": case["latest_tool_arm"],
        }
        for selector, name in choices.items():
            if not (valid["original"] and valid[name]):
                continue
            edited = arms[name]["action"]
            changed = {"primary": edited["write"] != original["write"], "secondary": edited != original}
            for metric, change in changed.items():
                result = counts[metric][selector]
                result["paired_scorable"] += 1
                result["paired_changed"] += int(change)
                result["changed"] += int(complete and change)

        withdrawal = arms["withdrawal"]["action"]
        hold = withdrawal["kind"]
        if valid["withdrawal"]:
            hold = "write" if withdrawal["write"] else "nonwrite"
        counts["withdrawal"][hold] += 1
        if original["write"] is True:
            counts["withdrawal_from_valid_original_write"][hold] += 1

        historical = case["historical"]
        fidelity = counts["historical"]
        fidelity["valid_baselines"] += int(valid["original"])
        fidelity["same_response_kind"] += int(valid["original"] and historical["kind"] == original["kind"])
        fidelity["both_text"] += int(historical["kind"] == original["kind"] == "text")
        if historical["kind"] == "tools":
            expected_names = [call["name"] for call in historical["calls"]]
            actual_names = [call["name"] for call in original["calls"]]
            fidelity["historical_tool_turns"] += 1
            fidelity["same_tool_names"] += int(actual_names == expected_names)
            fidelity["exact_ordered_calls"] += int(original == historical)

        cost = counts["cost"]
        for arm in arms.values():
            cost["generations"] += 1
            cost["generation_input_tokens"] += arm["input_tokens"]
            cost["generated_tokens"] += arm["generated_tokens"]
            cost["decode_seconds"] += arm["decode_seconds"]
            if "probe_seconds" in arm:
                cost["probe_calls"] += 1
                cost["probe_input_tokens"] += arm["input_tokens"]
                cost["probe_seconds"] += arm["probe_seconds"]
    return counts


def head_to_head(cases):
    """Compare ranked and baseline choices only when all three proposals can be scored."""
    pairs = {}
    for baseline in ("first", "latest_message", "latest_tool"):
        outcomes = {metric: Counter() for metric in ("write_changed", "action_changed")}
        for case in cases:
            selected = {"first": "policy", "latest_message": "user", "latest_tool": case["latest_tool_arm"]}
            arms = case["arms"]
            original, ranked, comparison = (
                arms[key]["action"] for key in ("original", case["ranked"][0], selected[baseline])
            )
            if any(action["write"] is None for action in (original, ranked, comparison)):
                for counts in outcomes.values():
                    counts["unscorable"] += 1
                continue
            for metric in outcomes:
                if metric == "write_changed":
                    ranked_changed = ranked["write"] != original["write"]
                    comparison_changed = comparison["write"] != original["write"]
                else:
                    ranked_changed = ranked != original
                    comparison_changed = comparison != original
                label = {(False, False): "neither", (True, True): "both",
                         (True, False): "ranked_only", (False, True): "baseline_only"}
                outcomes[metric][label[ranked_changed, comparison_changed]] += 1
        pairs[baseline] = outcomes
    return pairs


def selection():
    raw = (HERE / "selection.json").read_bytes()
    if sha256(raw).hexdigest() != SELECTION_SHA:
        raise ValueError("Selection changed since the protocol freeze.")
    spec = json.loads(raw)
    if spec["protocol_sha256"] != digest((HERE / "protocol.md").read_text()):
        raise ValueError("Protocol changed.")
    return spec


def verify(data):
    spec = selection()
    if (data["runtime"], data["revision"], data["model"]) != (RUNTIME, REVISION, demo.MODEL):
        raise ValueError("The saved run does not identify the registered model and runtime.")
    expected_tasks = [case["task"] for case in spec["cases"]]
    if data["selection_sha256"] != SELECTION_SHA or [case["task"] for case in data["cases"]] != expected_tasks:
        raise ValueError("Cohort differs from the frozen checkpoints.")
    for case, location in zip(data["cases"], spec["cases"]):
        arms = case["arms"]
        sources = list(location["anchors"])
        expected_order = sorted(sources, key=lambda name: -arms[name]["js_bits"])
        if (set(arms) != {"original", "withdrawal", *sources}
                or case["latest_tool_arm"] != location["latest_tool_arm"]
                or expected_order != case["ranked"]):
            raise ValueError("Arm inventory or ranked order changed.")
        for arm in arms.values():
            if "js_bits" in arm and not 0 <= arm["js_bits"] <= 1:
                raise ValueError("Jensen–Shannon divergence in bits must lie between zero and one.")
            if arm["action"]["kind"] in ("invalid", "truncated") and arm["action"]["write"] is not None:
                raise ValueError("Invalid or incomplete proposals must have no measured write status.")
            reached_limit = arm["stop"] == "token_limit"
            if (not 1 <= arm["generated_tokens"] <= 128 or arm["stop"] not in ("token_limit", "eos")
                    or (reached_limit and (arm["generated_tokens"] != 128 or arm["action"]["kind"] != "truncated"))):
                raise ValueError("A token count or output disposition is inconsistent.")
    if summary(data["cases"]) != data["summary"]:
        raise ValueError("Reported outcomes or cost disagree with the paired measurements.")


def repeat_case(snapshot, packet, tools, kinds, saved):
    demo.verify_checkpoint(snapshot)
    runtime = load_runtime(snapshot)
    if runtime.identity != RUNTIME:
        raise ValueError("Use the pinned CPU float32 runtime for the repeat.")
    with runtime.torch.inference_mode():
        ranked, arms = measure(runtime, packet["arms"], tools, kinds, None)
    if ranked != saved["ranked"]:
        raise ValueError("Repeat ranking differs.")
    for name, arm in arms.items():
        expected = saved["arms"][name]
        if any(value != expected[key] for key, value in arm.items() if not key.endswith("_seconds")):
            raise ValueError(f"Repeat differs on arm {name}")
    cost = Counter()
    for arm in arms.values():
        cost.update(generations=1, generation_input_tokens=arm["input_tokens"],
                    generated_tokens=arm["generated_tokens"], decode_seconds=arm["decode_seconds"])
        if "probe_seconds" in arm:
            cost.update(probe_calls=1, probe_input_tokens=arm["input_tokens"], probe_seconds=arm["probe_seconds"])
    return {"task": packet["task"], "identical_arms": len(arms), "cost": cost}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--audit", type=Path, help="Optional local-only raw generations for auditing dispositions.")
    parser.add_argument("--check", type=Path)
    parser.add_argument("--repeat-task", help="Regenerate every arm of one saved task and compare its measurements.")
    args = parser.parse_args()
    if args.repeat_task and not (args.check and args.data and args.snapshot):
        parser.error("--repeat-task requires --check, --data and a local --snapshot.")
    if args.check:
        if args.output or ((args.snapshot or args.audit) and not args.data):
            parser.error("--check accepts --data, plus an optional local --snapshot or --audit.")
        data = json.loads(args.check.read_text())
        verify(data)
        report = {"summary": data["summary"], "head_to_head": head_to_head(data["cases"])}
        if args.data:
            manifest, packets, tools, kinds = cohort(args.data)
            if manifest != selection():
                raise ValueError("Input reconstruction differs from the frozen manifest.")
            if args.repeat_task and args.repeat_task not in {p["task"] for p in packets}:
                parser.error("Unknown task in this cohort.")
            audit = json.loads(args.audit.read_text()) if args.audit else None
            tokenizer = None
            if args.snapshot:
                from transformers import AutoTokenizer

                if sha256((args.snapshot / "tokenizer_config.json").read_bytes()).hexdigest() != EXTRA_MODEL_FILES["tokenizer_config.json"]:
                    raise ValueError("Tokenizer template differs from the pinned release.")
                tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
            for packet, case in zip(packets, data["cases"]):
                if action(packet["next_calls"], kinds) != case["historical"]:
                    raise ValueError("Historical next action differs from the original trajectory.")
                if audit is not None:
                    for key, arm in case["arms"].items():
                        raw = audit[case["task"]][key]
                        if digest(raw) != arm["output_sha256"] or classify(raw, arm["stop"], tools, kinds) != arm["action"]:
                            raise ValueError("Raw model output differs from the recorded outcome.")
                if tokenizer is not None:
                    for name, public_messages in packet["arms"].items():
                        prompt = tokenizer.apply_chat_template(
                            public_messages, tools=tools, tokenize=False, add_generation_prompt=True
                        )
                        ids = tokenizer.encode(prompt, add_special_tokens=False)
                        arm = case["arms"][name]
                        if digest(prompt) != arm["prompt_sha256"] or len(ids) != arm["input_tokens"]:
                            raise ValueError("Rendered prompt hash or token count differs from the local run.")
                if packet["task"] == args.repeat_task:
                    report["repeat"] = repeat_case(args.snapshot, packet, tools, kinds, case)
        print(json.dumps(report, indent=2))
        return
    if not all((args.snapshot, args.data, args.output)) or args.output.exists() or (args.audit and args.audit.exists()):
        parser.error("Supply --data, --snapshot and unused output paths.")
    manifest, packets, tools, kinds = cohort(args.data)
    if manifest != selection():
        raise ValueError("Dataset did not reconstruct the frozen selection.")
    demo.verify_checkpoint(args.snapshot)
    for name, expected in EXTRA_MODEL_FILES.items():
        if sha256((args.snapshot / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Checkpoint mismatch for {name}")
    runtime = load_runtime(args.snapshot)
    if runtime.identity != RUNTIME:
        raise ValueError("Use the pinned CPU float32 runtime.")
    cases, audit = [], {}
    with runtime.torch.inference_mode():
        for pkt, location in zip(packets, manifest["cases"]):
            tid = pkt["task"]
            audit[tid] = {}
            ranked, arms = measure(runtime, pkt["arms"], tools, kinds, audit[tid] if args.audit else None)
            cases.append({"task": tid, "ranked": ranked, "latest_tool_arm": location["latest_tool_arm"],
                          "historical": action(pkt["next_calls"], kinds), "arms": arms})
            print(f"Completed task {tid} with {len(arms)} arms.", flush=True)
    data = {"model": demo.MODEL, "revision": REVISION, "runtime": RUNTIME, "selection_sha256": SELECTION_SHA,
          "cases": cases, "summary": summary(cases)}
    verify(data)
    with args.output.open("x") as f:
        json.dump(data, f, indent=2, allow_nan=False)
        f.write("\n")
    if args.audit:
        with args.audit.open("x") as f:
            json.dump(audit, f, ensure_ascii=False)
            f.write("\n")


if __name__ == "__main__":
    main()
