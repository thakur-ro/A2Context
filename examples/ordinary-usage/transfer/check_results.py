"""Recount published contract checks and paired interventions without a model or database."""

from collections import Counter
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
SEEDS = (11, 29, 47, 71, 89)


def read_rows(name):
    return [json.loads(line) for line in (HERE / name).read_text().splitlines()]


def check_pairs(rows, refs, arms):
    expected = {(session, call, seed): 1 for session, call in refs for seed in SEEDS}
    assert Counter((r["session"], r["call"], r["seed"]) for r in rows) == expected
    for row in rows:
        assert sorted(row["order"]) == sorted(arms)
        assert all(row[a]["http"] == 200 for a in arms)


def main():
    sessions = json.loads((HERE / "outcomes.json").read_text())["sql"]["sessions"]
    calls = {(s["session"], s["trace"], c["span"]) for s in sessions for c in s["calls"]}
    proposals = read_rows("proposal-checks.jsonl")
    assert len(proposals) == len({(r["call"], r["decision"]) for r in proposals}) == 14
    missing = set()
    siblings = set()
    for row in proposals:
        assert (row["session"], row["trace"], row["call"]) in calls
        verdict = row["check"]
        if row["framework_errors"]:
            assert verdict["status"] == "missing_required" and verdict["argument_pointer"] == "/query"
            assert row["framework_errors"] == [{"location":["query"], "type":"missing"}]
            ref = (row["session"], row["call"])
            missing.add(ref)
            if verdict["other_declarations"]:
                assert verdict["other_declarations"][0]["tool"] == "sql_db_schema"
                siblings.add(ref)
        else:
            assert verdict["status"] == "no_missing_field_observed"
    assert len(missing) == 4 and len(siblings) == 3

    repair = read_rows("repair-results.jsonl")
    arms = ("original", "generic", "specific")
    check_pairs(repair, missing, arms)
    assert [sum(r[a]["result"] == "valid_proposal" for r in repair) for a in arms] == [2, 8, 14]
    assert sum(r["generic"]["result"] == "valid_proposal" and r["specific"]["result"] == "valid_proposal"
               for r in repair) == 8
    errors = [r[a]["compile_error"] for r in repair for a in arms if r[a]["result"] == "valid_proposal"]
    assert len(errors) == 24 and sum(e.startswith("no such column:") for e in errors) == 19
    assert sum(e.startswith("no such table:") for e in errors) == 5

    experiment = read_rows("sibling-field-results.jsonl")
    rows = [r for r in experiment if "seed" in r]
    check_pairs(rows, siblings, ("original", "repeat", "edit"))
    assert [r["session"] for r in experiment if "excluded" in r] == ["sql-1"]
    assert all(r["original"]["proposals"] == r["repeat"]["proposals"] for r in rows)
    assert sum(r["original"]["old"] for r in rows) == 1
    assert not any(r["edit"]["alias"] or r["edit"]["old"] for r in rows)
    print("Verified 14 framework verdicts, 60 repair replies and 45 catalog-edit replies.")


if __name__ == "__main__":
    main()
