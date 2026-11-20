"""Replay the saved generations with and without the Qwen template's brace normalization."""
from collections import Counter
import argparse
import json

from study import HERE, adapter_choice, check, compare as compare_edits, study_design, trial_dispatch


def compare(data, design):
    check(data, design)
    target = design["pending_arguments"]["order_id"]
    counts = Counter()
    changed = []
    strict_cases = []
    for row in data["cases"]:
        strict_arms = {}
        for name, arm in row["arms"].items():
            counts["generations"] += 1
            parse, choice = adapter_choice(arm["raw"], arm["stop"], normalize_template_braces=False)
            counts["strict_parse_errors"] += parse == "error"
            counts["truncated"] += parse == "truncated"
            counts["strict_tool_calls"] += parse == "tool"
            counts["adapter_tool_calls"] += arm["parse"] == "tool"
            receipts = trial_dispatch(choice, row["approved"], design)
            strict_arms[name] = receipts
            for gate in ("without_gate", "with_gate"):
                original = arm[gate]["orders"][target]
                strict = receipts[gate]["orders"][target]
                counts[gate + "_state_differences"] += original != strict
                if original != strict:
                    changed.append({"case": row["id"], "arm": name, "gate": gate,
                                    "adapter_state": original, "strict_state": strict})
        strict_cases.append({**row, "arms": strict_arms})
    return {"counts": counts, "state_differences": changed,
            "strict_edit_results": compare_edits(strict_cases, design)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify the counts published for the frozen fixture")
    args = parser.parse_args()
    report = compare(json.loads((HERE / "results.json").read_text()), study_design())
    if args.check:
        expected = {"generations": 40, "truncated": 0, "adapter_tool_calls": 30,
                    "strict_parse_errors": 22, "strict_tool_calls": 8,
                    "without_gate_state_differences": 22, "with_gate_state_differences": 12}
        if report["counts"] != expected:
            raise ValueError(f"Published parser counts changed: {report['counts']}")
        print("Verified both parser policies on all 40 saved generations.")
    else:
        print(json.dumps(report, indent=2))
