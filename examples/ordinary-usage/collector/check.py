"""Recount the frozen pilot, controlled edits and prospective follow-up without inference."""

import argparse
from collections import Counter
from datetime import datetime
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name):
    return json.loads((HERE / name).read_text())


def verify_capture_offsets(folders, rows):
    from shared_text import experiment, final_calls

    recorded = [r for r in rows if "status" in r]
    regenerated = [r for folder in folders for r in experiment(folder, None)]
    assert sorted(recorded, key=lambda r: r["trace"]) == sorted(regenerated, key=lambda r: r["trace"])
    calls = {c["id"]: c for folder in folders for _, _, c in final_calls(folder)}

    def fragment(text, interval):
        start, end = interval
        return text.encode()[start:end].decode()

    for row in regenerated:
        if row["status"] != "candidate":
            continue
        call, prior = calls[row["call"]], calls[row["prior_call"]]
        current, earlier = [
            json.loads(c["recorded_request"]["body"])["messages"] for c in (call, prior)
        ]
        for key, match in row["matches"].items():
            assert fragment(call["decisions"][0]["outcome"], match["output_bytes"]) == key
            for where in match["locations"]:
                i = where["message"]
                assert fragment(current[i]["content"], where["bytes"]) == key
                assert fragment(earlier[i]["content"], where["prior_bytes"]) == key


def main(folders):
    for run in load("outcomes.json").values():
        assert run["distinct_findings"] == 0 and not run["capture_errors"]
        assert len(run["quiet_sample"]) == 3 and len(run["sessions"]) == 5
        for session in run["sessions"]:
            (call,) = session["calls"]
            assert session["exit"] == 0 and call["http"] == 200 and call["output_captured"]
            assert not call["new_findings"]
        assert run["score"]["pass_pct"] is None

    rows = [json.loads(line) for line in (HERE / "shared-text-results.jsonl").read_text().splitlines()]
    assert Counter(r["status"] for r in rows if "status" in r) == {
        "unassessed": 15,
        "no_comparison": 3,
        "no_match": 4,
        "candidate": 8,
    }
    if folders:
        verify_capture_offsets(folders, rows)
    candidates = {r["call"]: r for r in rows if r.get("status") == "candidate"}
    trials = [r for r in rows if "seed" in r]
    assert Counter((r["call"], r["seed"]) for r in trials) == {
        (call, seed): 1 for call in candidates for seed in (11, 29, 47)
    }
    switches = Counter()
    for r in trials:
        candidate = candidates[r["call"]]
        assert candidate["trace"] == r["trace"]
        assert sorted(r["order"]) == ["edit", "original", "repeat"]
        assert all(r[arm]["http"] == 200 for arm in r["order"])
        controls = set(r["original"]["original"]) & set(r["repeat"]["original"])
        assert controls and not (
            r["original"]["alias"] or r["repeat"]["alias"] or r["edit"]["original"]
        )
        switches[candidate["workflow"]] += any(
            candidate["aliases"][k] in r["edit"]["alias"] for k in controls
        )
    assert switches == {"papers": 10, "citations": 12}

    result = load("followup-results.json")
    assert result["application_exits"] == [0] * 5 and not result["capture_errors"]
    (card,) = result["final_groups"]
    assert card["occurrences"] == card["threads"] == 4 and card["kind"] == "shared_identifier"
    sessions = result["sessions"]
    first = result["first_evidence"]
    assert first["trace_id"] == sessions[1]["trace"] and first["call_id"] == sessions[1]["call"]
    assert first["detail"]["peer"]["trace"] == sessions[0]["trace"]
    for row in sessions:
        assert row["http"] == 200 and len(row["retrieved_labels"]) == 3
        assert all(label.endswith("/README.md") for label in row["retrieved_labels"])
        assert set(row["example_labels"]).isdisjoint(row["retrieved_labels"])
        assert set(row["application_sources"].split(", ")) & set(row["example_labels"])
    for match in first["detail"]["matches"]:
        start, end = match["output_bytes"]
        assert sessions[1]["reply"].encode()[start:end].decode() == match["key"]
        assert match["key"] in sessions[1]["example_labels"] and match["key"] in sessions[1][
            "application_sources"
        ].split(", ")
    times = [datetime.fromisoformat(result[k]) for k in ("started", "displayed", "developer_reviewed")]
    assert times == sorted(times)
    print(
        f"Verified ten frozen sessions, 72 replay replies and one finding in five fresh sessions. "
        f"First displayed after {(times[1] - times[0]).total_seconds():.2f} seconds."
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--captures",
        nargs="+",
        type=Path,
        help="Six original workflow directories to check UTF-8 offsets",
    )
    main(parser.parse_args().captures)
