"""Repeat only censored baseline generations at a fixed second output limit."""
import argparse
import json
from pathlib import Path

from inputs import HERE, cohort, digest
from run import EXTRA_MODEL_FILES, action, classify, demo, load_runtime, selection, verify
from study import generate_tokens

LIMIT = 256
ORIGINAL = HERE / "results.json"


def verify_budget(saved, original):
    previous = {c["task"]: c for c in original["cases"] if c["arms"]["original"]["stop"] == "token_limit"}
    if (saved["source_results_sha256"] != digest(ORIGINAL.read_text()) or saved["limit"] != LIMIT
            or [row["task"] for row in saved["cases"]] != list(previous)):
        raise ValueError("Budget, cohort or original results differ.")
    for row in saved["cases"]:
        case = previous[row["task"]]
        baseline = case["arms"]["original"]
        stop = row["stop"]
        if (row["prompt_sha256"] != baseline["prompt_sha256"] or row["prefix_sha256"] != baseline["output_sha256"]
                or stop not in ("token_limit", "eos") or not baseline["generated_tokens"] < row["generated_tokens"] <= LIMIT
                or (stop == "token_limit") != (row["action"]["kind"] == "truncated")
                or (stop == "token_limit" and row["generated_tokens"] != LIMIT)):
            raise ValueError(f"Invalid continuation for {row['task']}")
        if row["historical"] != case["historical"]:
            raise ValueError("Historical action changed.")


def measure(data, snapshot, original):
    from hashlib import sha256
    manifest, packets, tools, kinds = cohort(data)
    if manifest != selection():
        raise ValueError("The requests differ from the frozen cohort.")
    demo.verify_checkpoint(snapshot)
    for name, expected in EXTRA_MODEL_FILES.items():
        if sha256((snapshot / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Checkpoint mismatch for {name}")
    runtime = load_runtime(snapshot)
    if runtime.identity != original["runtime"]:
        raise ValueError("The original CPU runtime is required.")
    rows = []
    with runtime.torch.inference_mode():
        for packet, case in zip(packets, original["cases"], strict=True):
            prior = case["arms"]["original"]
            if prior["stop"] != "token_limit":
                continue
            prompt, ids, _, _ = runtime.prompt_input(packet["arms"]["original"], tools)
            if digest(prompt) != prior["prompt_sha256"] or len(ids) != prior["input_tokens"]:
                raise ValueError("Prompt differs from the original experiment.")
            tokens = generate_tokens(runtime, ids, LIMIT)
            eos = tokens[-1] in runtime.generation_config.eos_token_id
            stop = "eos" if eos else "token_limit"
            raw = runtime.tokenizer.decode(tokens[:-1] if eos else tokens, skip_special_tokens=False)
            prefix = runtime.tokenizer.decode(tokens[:prior["generated_tokens"]], skip_special_tokens=False)
            rows.append({"task": case["task"], "prompt_sha256": digest(prompt), "prefix_sha256": digest(prefix),
                         "output_sha256": digest(raw), "generated_tokens": len(tokens), "stop": stop,
                         "action": classify(raw, stop, tools, kinds), "historical": action(packet["next_calls"], kinds)})
            print(f"Checked baseline on task {case['task']}", flush=True)
    return {"source_results_sha256": digest(ORIGINAL.read_text()), "limit": LIMIT, "cases": rows}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path)
    parser.add_argument("--snapshot", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--output", type=Path)
    mode.add_argument("--check", type=Path)
    args = parser.parse_args()
    original = json.loads(ORIGINAL.read_text())
    verify(original)
    if args.check:
        verify_budget(json.loads(args.check.read_text()), original)
        print("Verified the longer-budget baseline inventory and all four saved prefixes.")
    else:
        if not args.data or not args.snapshot or args.output.exists():
            parser.error("Use --data, --snapshot and an unused output path.")
        result = measure(args.data, args.snapshot, original)
        verify_budget(result, original)
        with args.output.open("x") as out:
            out.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
