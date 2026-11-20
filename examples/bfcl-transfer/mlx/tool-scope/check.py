"""Verify paired tool-scope receipts and print the counts used in the study."""
from collections import Counter, defaultdict
from itertools import chain
from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def summarize(report):
    totals = defaultdict(Counter)
    for case in report["cases"]:
        ref = case["reference"]
        assert ref["usable"] and all(e["in_scope"] and e["status"] == "executed" for e in ref["calls"])
        scoped_count = 4 if case["group"] == "booking_control" else 3
        full_catalog, scoped_catalog = case["catalogs"]["full"], case["catalogs"]["scoped"]
        assert full_catalog["captured"] == 18 and len(full_catalog["outside"]) == 18 - scoped_count
        assert scoped_catalog["captured"] == scoped_count and not scoped_catalog["outside"]
        assert not full_catalog["unclassified"] and not scoped_catalog["unclassified"]
        base = case["arms"]["full"]
        gate = case["arms"]["gate"]
        assert base["generations"][0] == gate["generations"][0]
        assert base["generations"][0]["prompt_sha256"] != case["arms"]["scoped"]["generations"][0]["prompt_sha256"]
        if all(e["in_scope"] for e in base["calls"]):
            assert gate == base
        if case["group"] == "booking_control":
            blocked = case["quote_on_booking"]
            assert not blocked["state_match"] and not blocked["required_actions"]
            assert any(e["name"] == "book_flight" and e["status"] == "outside_workflow_scope" for e in blocked["calls"])
        for arm, row in case["arms"].items():
            counts = totals[case["group"], arm]
            before = ref["calls"][0]["before"]
            for e in row["calls"]:
                assert e["before"] == before
                changed = e["after"] != before
                before = e["after"]
                assert (e["status"] == "outside_workflow_scope") == (arm != "full" and not e["in_scope"])
                if e["status"] == "outside_workflow_scope":
                    assert not changed and "response_sha256" not in e
                counts["proposals"] += 1
                counts["outside"] += not e["in_scope"]
                counts["outside_mutations"] += not e["in_scope"] and changed
                counts["allowed_mutations"] += e["in_scope"] and changed
            assert before == row["state_sha256"]
            assert row["usable"] == (row["stop"] == "normal" and
                    all(row[k] for k in ("required_actions", "required_responses", "state_match")))
            counts["cases"] += 1
            counts["normal"] += row["stop"] == "normal"
            for key in ("required_actions", "required_responses", "state_match", "usable"):
                counts[key] += row[key]
    return totals


def main():
    results = HERE / "results.json"
    report = json.loads(results.read_text())
    repeat = json.loads((HERE / "verification.json").read_text())
    assert report["protocol_sha256"] == sha256((HERE / "protocol.md").read_bytes()).hexdigest()
    assert report["model_files_sha256"] == sha256((HERE.parent / "files.json").read_bytes()).hexdigest()
    assert repeat["report_sha256"] == sha256(results.read_bytes()).hexdigest()
    ids = [r["id"] for r in report["cases"]]
    assert ids == repeat["matched"] == [f"multi_turn_base_{n}" for n in (167,182,192,194,150,155,163,179)]
    generations = list(chain.from_iterable(arm["generations"] for case in report["cases"] for arm in case["arms"].values()))
    print("Generations:", len(generations), "input tokens:", sum(g["input_tokens"] for g in generations),
          "output tokens:", sum(g["generated_tokens"] for g in generations))
    for (group, arm), counts in summarize(report).items():
        print(group, arm, dict(counts))


if __name__ == "__main__":
    main()
