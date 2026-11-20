"""Check the archived experiment without a model download or GPU."""
from collections import Counter
from hashlib import sha256
from itertools import combinations
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ARMS = ("full", "scoped", "dots", "letters")


def signature(event):
    return event["name"], event.get("arguments_sha256")


def verify():
    path = HERE / "results.json"
    report = json.loads(path.read_text())
    cases = report["cases"]
    ids = [f"multi_turn_base_{n}" for n in (116, 117, 118, 126, 127, 132, 135, 142, 146)]
    assert [c["id"] for c in cases] == ids
    assert report["protocol_sha256"] == sha256((HERE / "protocol.md").read_bytes()).hexdigest()
    assert report["model_files_sha256"] == sha256((HERE.parent / "files.json").read_bytes()).hexdigest()
    assert json.loads((HERE / "verification.json").read_text()) == {
        "report_sha256": sha256(path.read_bytes()).hexdigest(), "matched": ids}
    counts = {a: Counter() for a in ARMS}
    for case in cases:
        ref, no_call = case["positive"], case["no_call"]
        assert ref["usable"] and not no_call["usable"] and not no_call["calls"]
        assert all(e["status"] == "executed" for e in ref["calls"])
        assert set(case["arms"]) == set(case["opener_probability"]) == set(ARMS)
        assert set(case["js_bits"]) == {f"{a}/{b}" for a, b in combinations(ARMS, 2)}
        assert all(0 <= n <= 1 for field in ("opener_probability", "js_bits") for n in case[field].values())
        for arm in ARMS:
            row = case["arms"][arm]
            calls = row["calls"]
            state = no_call["state_sha256"]
            for call in calls:
                assert call["before"] == state
                state = call["after"]
            assert row["state_sha256"] == state
            assert row["state_match"] == (state == ref["state_sha256"])
            assert row["exact_calls"] == (list(map(signature, calls)) == list(map(signature, ref["calls"])))
            required = Counter(e["response_sha256"] for e in ref["calls"])
            received = Counter(e["response_sha256"] for e in calls if e["status"] == "executed")
            assert row["required_responses"] == (not (required - received))
            success = row["exact_calls"] and row["state_match"] and row["required_responses"] and row["stop"] == "normal"
            assert row["usable"] == success
            gens = row["generations"]
            assert 1 <= len(gens) <= 3 and len(calls) <= 6
            for gen in gens:
                assert gen["full_catalog_same_history_tokens"] - gen["input_tokens"] == (2465 if arm == "scoped" else 0)
            totals = counts[arm]
            totals["complete"] += success
            totals["matched_state"] += row["state_match"]
            totals["recoveries"] += success and not case["arms"]["full"]["usable"]
            totals["losses"] += not success and case["arms"]["full"]["usable"]
            totals["generations"] += len(gens)
            totals["input_tokens"] += sum(g["input_tokens"] for g in gens)
            totals["output_tokens"] += sum(g["generated_tokens"] for g in gens)
    for arm, count in counts.items():
        print(arm, dict(count))
    return counts


if __name__ == "__main__":
    verify()
