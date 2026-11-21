"""Recount published pilot outcomes and resolve evidence links without inference."""

from collections import Counter
import argparse
from hashlib import sha256
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent


def read(name):
    return json.loads((HERE / name).read_text())


def check_blinded(study, sessions, expected_arms):
    rows = read(f"{study}-arms.json")
    reviews = read(f"{study}-review.json")["reviews"]
    assert len(rows) == len(reviews) and {row["id"] for row in rows} == reviews.keys()
    covered = Counter()
    for row in rows:
        assert row["met"] is reviews[row["id"]]["met" if study == "summary" else "meets_criterion"]
        assert not row["met"] and reviews[row["id"]]["answer"].strip()
        covered[row.get("session", "papers-4"), row["seed"], row["arm"]] += 1
    assert covered == {(s, seed, a): 1 for s in sessions for seed in (11, 29) for a in expected_arms}


def main(log_directory=None):
    sessions = {s["session"]: s for s in read("first-pass.json")["sessions"]}
    assert set(sessions) == {f"{w}-{i}" for w in ("papers", "tools") for i in range(1, 6)}
    refs = set()
    for workflow, count in (("papers", 20), ("tools", 8)):
        runs = [s for name, s in sessions.items() if name.startswith(workflow)]
        assert sum(len(s["responses"]) for s in runs) == count
        for s in runs:
            assert s["exit"] == 0
            for response in s["responses"]:
                assert response["http"] == 200 and response["output_captured"] and not response["visible_cards"]
                refs.add((s["trace"], response["span"]))
        sample = sorted(runs, key=lambda s: sha256(s["trace"].encode()).digest())[:3]
        review = (HERE / "quiet-review.md").read_text()
        for s in sample:
            assert any(r["span"] in review for r in s["responses"])
    for row in read("retrieval-candidates.json"):
        candidate = row["candidate"]
        if candidate:
            assert (candidate["trace"], candidate["call"]) in refs
            assert sessions[candidate["session"]]["closed"] < sessions[row["session"]]["started"]
            assert candidate["source"] not in row["retrieved"]
    check_blinded("passage", ("papers-2", "papers-4"), ("original", "repeat", "candidate", "control"))
    check_blinded("summary", ("papers-4",), ("original", "repeat", "passage", "control"))
    rows = [json.loads(line) for line in (HERE / "earlier-schema-results.jsonl").read_text().splitlines()]
    assert [r["session"] for r in rows if r.get("eligible") is False] == [f"tools-{n}" for n in range(1, 5)]
    trials = [r for r in rows if "seed" in r]
    assert sorted(r["seed"] for r in trials) == list(range(1, 16))
    for r in trials:
        assert r["session"] == "tools-5" and (r["witness"]["trace"], r["witness"]["call"]) in refs
        for arm in ("original", "repeat", "candidate", "control"):
            assert r[arm]["status"] == 200 and r[arm]["advertised_proposal"] is False
    rows = [json.loads(line) for line in (HERE / "citation-example-results.jsonl").read_text().splitlines()]
    papers = [s for name, s in sessions.items() if name.startswith("papers")]
    assert Counter((r["session"], r["seed"]) for r in rows) == {
        (s["session"], seed): 1 for s in papers for seed in (11, 29, 47)}
    switches = 0
    for row in rows:
        assert row["call"] == sessions[row["session"]]["responses"][-1]["span"]
        assert set(row["order"]) == {"original", "repeat", "edit"}
        old, new, passages = set(row["swaps"]), set(row["swaps"].values()), set(row["passages"])
        assert len(old) == len(new) == 2 and len(passages) == 3 and len(old | new | passages) == 7
        for arm in row["order"]:
            assert row[arm]["http"] == 200 and passages.intersection(row[arm]["citations"])
        original, repeat, edited = (set(row[arm]["citations"]) for arm in ("original", "repeat", "edit"))
        assert original == repeat and not new & original and not old & edited and new & edited
        switches += any(k in original and v in edited for k, v in row["swaps"].items())
    assert switches == 14
    displayed = read("displayed-citations.json")
    assert {r["session"] for r in displayed} == {s["session"] for s in papers}
    assert sum(len(r["matches"]) for r in displayed) == 7
    for row in displayed:
        assert (row["trace"], row["call"]) in refs and row["matches"]
        assert row["application_log_sha256"] == sessions[row["session"]]["application_log_sha256"]
        for match in row["matches"]:
            assert len(match["prefix"]) == 64 and match["application_text"] == "()"
            if log_directory:
                content = (log_directory / (row["session"] + ".log")).read_bytes()
                assert sha256(content).hexdigest() == row["application_log_sha256"]
                prefix = match["prefix"].encode()
                offset = match["application_log_utf8_offset"]
                assert content[offset-len(prefix):offset+2] == prefix + b"()"
    transfer = read("transfer/outcomes.json")
    for workflow, count in (("sql",20), ("citations",5)):
        outcome = transfer[workflow]
        assert outcome["distinct_findings"] == 0 and not outcome["capture_errors"]
        runs = outcome["sessions"]
        assert len(runs) == 5 and sum(len(s["calls"]) for s in runs) == count
        assert all(c["http"] == 200 and c["output_captured"] and not c["new_findings"] for s in runs for c in s["calls"])
        assert sum(s["exit"] != 0 for s in runs) == (4 if workflow == "sql" else 0)
        sample = sorted(runs,key=lambda s: sha256(s["trace"].encode()).digest())[:3]
        assert outcome["quiet_sample"] == [s["session"] for s in sample]
    rows = [json.loads(line) for line in (HERE / "transfer/example-edit-results.jsonl").read_text().splitlines()]
    assert Counter((r["session"],r["seed"]) for r in rows) == {
        (s["session"],seed): 1 for s in transfer["citations"]["sessions"] for seed in (11,29,47)}
    old_pairs, new_pairs = set(), set()
    for r in rows:
        assert all(r[a]["http"] == 200 for a in ("original","repeat","edit"))
        original, repeat, edit = (set(r[a]["terms"]) for a in ("original","repeat","edit"))
        assert original == repeat and original <= {"water","sky"} and edit <= {"wax","lamp"}
        if original:
            old_pairs.add((r["session"], r["seed"]))
        if edit:
            new_pairs.add((r["session"], r["seed"]))
    assert len(old_pairs) == len(new_pairs) == 2 and old_pairs.isdisjoint(new_pairs)
    print("Verified both ten-session pilots, their quiet samples, and the published replay counts.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--original-paper-logs", type=Path, help="Also resolve offsets in the original pilot logs")
    main(parser.parse_args().original_paper_logs)
