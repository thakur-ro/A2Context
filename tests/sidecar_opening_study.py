"""Exercise the scored-opening recommendation with saved, independently replayed answers."""
import argparse
from copy import deepcopy
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "scripts"), str(ROOT / "examples")]
from check_sidecar import get, send, send_traces, server
from view_opening_study import ORG, records, verified_report


def counts(row):
    return tuple(row[k] for k in ("scored", "tested", "affected"))


def check(binary, folder):
    db = folder / "opening.sqlite"
    cases = list(records())
    with server(binary, db, "--capture-content") as base:
        for trace, _, _ in cases:
            send_traces(base, trace)
        assert get(base, "impact")["opening_gap"] is None
        for trace, replay, outcomes in cases:
            if outcomes[()][1] is None:
                continue
            with TemporaryDirectory(dir=folder) as work:
                report = verified_report(binary, Path(work), trace, replay, outcomes)
            send(base, "/api/replay-reports", report, expected=201)
            send(base, "/api/replay-reports", report)
        gap = get(base, "impact", org=ORG)["opening_gap"]
        assert counts(gap) == (8, 5, 3) and gap["affected_inputs"] == 3
        selection = {k: gap[k] for k in ("group", "factor")}
        assert not get(base, "opening-coverage", **selection, user="another")["rows"]
        assert not get(base, "opening-coverage", **selection, after="2026-11-21")["rows"]
        policy, faq = get(base, "opening-sources", **selection)["rows"]
        assert (policy["source"], counts(policy)) == ("journal-policy", (8, 5, 2))
        assert (faq["source"], counts(faq)) == ("journal-faq", (7, 4, 1))
        examples = get(base, "opening-conversations", **selection, slot=policy["slot"])["rows"]
        assert [r["thread"] for r in examples] == ["Journal question 8", "Journal question 4"]
        for row in get(base, "opening-conversations", **selection)["rows"]:
            card, = get(base, "findings", **selection, trace=row["trace"], call=row["call"])["rows"]
            detail = card["evidence"]["detail"]
            assert any(c["context_id"] == row["context"] and c.get("changed") for c in detail["opening"]["checks"])
            token = detail["selector_check"]["token_contrast"]
            assert token["position"] == (8 if row["thread"] == "Journal question 4" else 4)

        # Redeliver the same input on a new trace in the same conversation, then in a new one.
        trace, replay, outcomes = deepcopy(cases[3])
        span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        for suffix, expected in [("a", (8, 5, 3)), ("b", (9, 6, 4))]:
            span["traceId"] = suffix * 32
            if suffix == "b":
                attr = next(a for a in span["attributes"] if a["key"] == "gen_ai.conversation.id")
                attr["value"]["stringValue"] = "Same question in a different conversation"
            replay["location"]["run_id"] = "otlp:" + span["traceId"]
            replay["id"] += suffix
            send_traces(base, trace)
            with TemporaryDirectory(dir=folder) as work:
                report = verified_report(binary, Path(work), trace, replay, outcomes)
            send(base, "/api/replay-reports", report, expected=201)
            result = get(base, "impact")["opening_gap"]
            assert counts(result) == expected and result["affected_inputs"] == 3

    with sqlite3.connect(db) as connection:
        connection.execute("UPDATE observations SET detail=json_remove(detail,'$.opening') WHERE kind='prefill'")
        connection.execute("PRAGMA user_version=25")
    with server(binary, db, "--capture-content") as base:
        assert get(base, "impact")["opening_gap"] == result
    with server(binary, folder / "redacted.sqlite") as base:
        send_traces(base, cases[3][0])
        assert get(base, "impact")["opening_gap"] is None
        assert not get(base, "opening-conversations", **selection)["rows"]


def measure(binary, folder):
    from sidecar_conflict_coverage import time_reads

    templates = []
    for trace, replay, outcomes in records():
        report = None
        if outcomes[()][1] is not None:
            with TemporaryDirectory(dir=folder) as work:
                report = verified_report(binary, Path(work), trace, replay, outcomes)
        templates.append((trace, report))
    with server(binary, folder / "cost.sqlite", "--capture-content") as base:
        for i in range(128):
            trace, report = deepcopy(templates[i % len(templates)])
            span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
            span["traceId"] = f"{i + 1:032x}"
            thread = next(a for a in span["attributes"] if a["key"] == "gen_ai.conversation.id")
            thread["value"]["stringValue"] = f"journal-{i:03}"
            send_traces(base, trace)
            if report:
                report["location"]["run_id"] = "otlp:" + span["traceId"]
                send(base, "/api/replay-reports", report, expected=201)
        gap = get(base, "impact")["opening_gap"]
        assert counts(gap) == (128, 80, 48) and gap["affected_inputs"] == 3
        selection = {k:gap[k] for k in ("group", "factor")}
        for route in ("impact", "opening-coverage", "opening-sources", "opening-conversations"):
            time_reads(base, route, {} if route == "impact" else selection)
        names, offset = set(), 0
        while offset is not None:
            page = get(base, "opening-conversations", offset=offset, **selection)
            names.update(r["thread"] for r in page["rows"])
            offset = page["next_offset"]
        assert len(names) == 48


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--measure", action="store_true", help="Time warm reads on 128 synthetic conversations")
    args = parser.parse_args()
    with TemporaryDirectory() as work:
        if args.measure:
            measure(args.binary.resolve(), Path(work))
        else:
            check(args.binary.resolve(), Path(work))
            print("Opening-token recommendation, source evidence, retries and migration passed.")
