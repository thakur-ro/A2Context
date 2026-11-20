"""Locate the first differing token for every held-opening miss in the journal holdout."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

from check_question_holdout import HERE, conditions, load_runtime, verify_model

sys.path.insert(0, str(HERE.parents[2] / "examples/public-qa"))
from inspect_rank_misses import measure_conditions
from view_passage_edits import token_contrast


def readouts(model):
    verify_model(model)
    runtime = load_runtime(model)
    tokenizer_hash = sha256((model / "tokenizer.json").read_bytes()).hexdigest()
    reference = json.loads((HERE / "case.json").read_text())
    cases = json.loads((HERE / "question-results.json").read_text())["cases"]
    result = {}
    with runtime.torch.inference_mode():
        for case in cases:
            scores, answers = case["scores"], case["answers"]
            if answers["original"] is None:
                continue
            top, lower = sorted(scores["effects"], key=lambda e: -e["js_bits"])
            if scores["tokenizer_sha256"] != tokenizer_hash or scores["runtime"] != runtime.identity:
                raise ValueError("Use the tokenizer and runtime of the original experiment.")
            changes = [e for e in (top, lower) if e["top_token_id"] == scores["top_token_id"]
                       and answers[e["source_id"]] is not None and answers[e["source_id"]] != answers["original"]]
            if not changes:
                continue
            if changes != [lower] or answers[top["source_id"]] != answers["original"]:
                raise ValueError("This three-condition readout requires an unchanged higher-ranked edit.")
            batch, _, _ = conditions(reference, case["question"])
            bodies = {e["source_id"]:e["request"] for e in batch["edits"]}
            arms = [("original",batch["baseline"], answers["original"], 0.0)]
            for label, edit in (("top",top), ("lower",lower)):
                name = edit["source_id"]
                arms.append((label,bodies[name],answers[name],edit["js_bits"]))
            row = measure_conditions(runtime, arms)
            for condition, opening in zip(row["conditions"], (scores,top,lower), strict=True):
                if condition["answer_token_ids"][0] != opening["top_token_id"]:
                    raise ValueError("The greedy rerun did not reproduce the recorded opening.")
            identity = {"model":scores["model_id"], "revision":scores["model_revision"], "runtime":runtime.identity,
                        "tokenizer_sha256":tokenizer_hash, "stop_ids":runtime.generation_config.eos_token_id}
            result[scores["baseline_request_sha256"]] = token_contrast(identity, row)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Choose a new output file.")
    result = readouts(args.model)
    with args.output.open("x") as output:
        json.dump(result, output, ensure_ascii=False, indent=2, allow_nan=False)
        output.write("\n")
