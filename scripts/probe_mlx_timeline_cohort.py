"""Repeat the local activation timeline on the declared synthetic conversations."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import sys

from probe_mlx_answer_timeline import probe


def evidence(template, case, conversation, report):
    run = deepcopy(template)
    run.update({key: conversation[key] for key in ("organization_id", "user_id", "conversation_id")})
    run["id"] = case["location"]["run_id"]
    call = run["calls"][0]
    call["started_at"] = conversation["started_at"]
    for item, message in zip(call["context"], case["request"]["messages"], strict=True):
        item["text"] = message["content"]
    for item in call["context"]:
        if item["id"] in {edit["context_id"] for edit in case["edits"]}:
            item.update(source_id=item["id"], source_version="demo-v1")
    call["decisions"][0]["outcome"] = report["baseline_answer"]
    return run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1] / "examples/local-mlx-timeline"
    cases = json.loads((root / "cohort.json").read_text())
    base = json.loads((root / "case.json").read_text())
    (template,) = json.loads((root / "evidence.json").read_text())
    reports, runs, failures = [], [], []
    for conversation in cases:
        identifier = conversation["conversation_id"]
        case = deepcopy(base)
        case["id"] = "lfm-" + identifier
        case["location"]["run_id"] = "local-model-" + identifier
        case["request"]["messages"][-1]["content"] = conversation["question"]
        try:
            report = probe(case, args.model, args.model_id, args.revision)
        except ValueError as error:
            failures.append({"conversation_id": identifier, "error": str(error)})
            print(f"{identifier}: {error}", file=sys.stderr)
            continue
        reports.append(report)
        runs.append(evidence(template, case, conversation, report))

    args.output.mkdir(parents=True)
    for filename, value in [
        ("activation-reports.json", reports), ("evidence.json", runs), ("failures.json", failures),
    ]:
        (args.output / filename).write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n")
    print(f"Measured {len(reports)} of {len(cases)} declared conversations.")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
