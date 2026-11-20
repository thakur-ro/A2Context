"""Check replay coverage across conversations and attribution to individual source edits."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import sys
from statistics import median
from tempfile import TemporaryDirectory
from time import perf_counter

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "scripts"), str(ROOT / "examples")]
from check_sidecar import get, send, send_traces, server
from view_conflict_coverage import sessions


def individual(report, arm):
    single = deepcopy(report)
    single["id"] += f"-arm-{arm}"
    single.pop("pair", None)
    single["variants"] = [single["variants"][arm]]
    return single


def attach(base, report):
    send(base, "/api/replay-reports", report, expected=201)


def counts(base, selection, **scope):
    row, = get(base, "conflict-coverage", **selection, **scope)["rows"]
    return tuple(row[key] for key in ("untested", "partial", "complete", "calls"))


def check(binary, folder):
    db = folder / "coverage.sqlite"
    first, second, third = sessions()
    with server(binary, db, "--capture-content") as base:
        for _, trace, _ in (first, second, third):
            send_traces(base, trace)
        assert get(base, "impact")["conflict_gap"] is None
        attach(base, first[2])
        send(base, "/api/replay-reports", first[2])
        attach(base, individual(second[2], 0))
        gap = get(base, "impact")["conflict_gap"]
        selection = {k:gap[k] for k in ("group","factor")}
        assert counts(base, selection) == (1, 1, 1, 3)
        assert counts(base, selection, user="support-17") == (0, 1, 1, 2)
        assert counts(base, selection, after="2026-11-21") == (1, 0, 0, 1)
        for scope in [{"org":"absent"}, {"workflow":"absent"}, {"meta_key":"stage","meta_value":"absent"}]:
            assert not get(base, "conflict-coverage", **selection, **scope)["rows"]
        attach(base, individual(second[2], 1))
        assert counts(base, selection) == (1, 1, 1, 3)
        row, = get(base, "conflict-conversations", coverage="partial", **selection)["rows"]
        assert row["thread"] == second[0] and len(row["tested_sources"]) == 2 and row["tested"] == 1

        # Another conflicting call in the first conversation has no tests.
        another = deepcopy(first[1])
        span = another["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        span["traceId"] = "a" * 32
        send_traces(base, another)
        send_traces(base, another)
        assert counts(base, selection) == (1, 2, 0, 4)
        members = get(base, "conflict-conversations", coverage="partial", **selection)["rows"]
        least = next(r for r in members if r["thread"] == first[0])
        assert least["tested"] == 0 and least["trace"] == "otlp:" + span["traceId"] and least["calls"] == 2

        # Identical rule and source IDs from another organization have a separate factor.
        outside = deepcopy(another)
        outside_span = outside["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        outside_span["traceId"] = "b" * 32
        next(a for a in outside_span["attributes"] if a["key"] == "nexusweave.organization.id")["value"]["stringValue"] = "outside"
        send_traces(base, outside)
        assert counts(base, selection) == (1, 2, 0, 4)
        assert get(base,"impact",org="outside")["conflict_gap"] is None

    with sqlite3.connect(db) as sql:
        sql.execute("UPDATE observations SET detail=json_remove(detail,'$.conflict_trials') WHERE kind='conflict'")
        sql.execute("PRAGMA user_version=23")
    with server(binary, db, "--capture-content") as base:
        assert counts(base, selection) == (1, 2, 0, 4)
        test = deepcopy(first[2])
        test["location"]["run_id"] = least["trace"]
        for report in [test, second[2], third[2]]:
            attach(base, report)
        assert counts(base, selection) == (0, 0, 3, 4)
        assert get(base, "impact")["conflict_gap"] is None


def measure(binary, folder):
    """Repeat three saved input bodies in 384 distinct synthetic conversations."""
    templates = list(sessions())
    with server(binary, folder / "cost.sqlite", "--capture-content") as base:
        for index in range(384):
            _, trace, report = deepcopy(templates[index % 3])
            span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
            span["traceId"] = f"{index + 1:032x}"
            thread = next(a for a in span["attributes"] if a["key"] == "gen_ai.conversation.id")
            thread["value"]["stringValue"] = f"cost-{index:03}"
            send_traces(base, trace)
            report["location"]["run_id"] = "otlp:" + span["traceId"]
            if index % 3 == 0:
                attach(base, report)
            elif index % 3 == 1:
                attach(base, individual(report, 0))
        gap = get(base, "impact")["conflict_gap"]
        selection = {key:gap[key] for key in ("group","factor")}
        assert counts(base, selection) == (128, 128, 128, 384)
        # Follow every page, including ties on recorded timestamps.
        names = []
        offset = 0
        while offset is not None:
            page = get(base,"conflict-conversations",coverage="partial",offset=offset,**selection)
            names.extend(r["thread"] for r in page["rows"])
            offset = page["next_offset"]
        assert len(names) == len(set(names)) == 128
        for route, params in [("impact",{}), ("conflict-coverage",selection),
                              ("conflict-conversations",{**selection,"coverage":"untested"})]:
            timings = []
            get(base,route,**params)
            for _ in range(30):
                start = perf_counter()
                get(base,route,**params)
                timings.append(1000 * (perf_counter() - start))
            print(json.dumps({"route":route, "requests":len(timings), "median_ms":round(median(timings),2),
                              "p95_ms":round(sorted(timings)[28],2)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--measure", action="store_true", help="Time warm reads across 384 synthetic conversations")
    args = parser.parse_args()
    with TemporaryDirectory() as tmp:
        if args.measure:
            measure(args.binary, Path(tmp))
        else:
            check(args.binary, Path(tmp))
            print("Verified conflict coverage, separate reports, late calls, scope and v23 recovery.")
