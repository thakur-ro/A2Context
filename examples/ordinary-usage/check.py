"""Recount published pilot outcomes and resolve evidence links without inference."""

from collections import Counter
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


def main():
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
    print("Verified ten pilot sessions, six quiet review references, 24 blinded outcomes and 60 proposal trials.")


if __name__ == "__main__":
    main()
