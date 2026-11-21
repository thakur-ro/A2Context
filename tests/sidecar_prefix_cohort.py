"""Verify cross-conversation source suggestions and their matched question pairs."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "examples"), str(ROOT / "scripts")]
from check_sidecar import get, sdk_sender, send, send_traces, server
from view_calendar_prefixes import CASES, ORG, trace_for


def span(trace):
    return trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]


def check(binary, folder, cases, sdk_python):
    db = folder / "checks.sqlite"
    second_sender = sdk_sender(sdk_python) if sdk_python else send_traces
    traces = [trace_for(case, i) for i, case in enumerate(cases, 1)]
    with server(binary, db, "--capture-content") as base:
        # Without a trial the first suggestion cannot supply a matched comparison.
        partial = deepcopy(traces[0])
        record = span(partial)
        record["attributes"] = [a for a in record["attributes"] if a["key"] != "a2context.shadow_probes"]
        send_traces(base, partial)
        for i, trace in enumerate(traces[1:]):
            (second_sender if i % 2 == 0 else send_traces)(base, trace)
        group, = get(base, "prefix-groups", org=ORG)["rows"]
        design = {"design": group["design"]}
        assert (group["conversations"], group["confirmed"], group["untested"]) == (8, 7, 1)
        assert len(get(base, "prefix-contrasts", **design)["rows"]) == 3

        # A later capture on a separate span in the same thread supplies the trials.
        complete = deepcopy(traces[0])
        span(complete)["traceId"] = "a" * 32
        send_traces(base, complete)
        send_traces(base, complete)
        group, = get(base, "prefix-groups", **design)["rows"]
        assert (group["conversations"], group["inputs"], group["confirmed"]) == (8, 8, 8)
        sources = get(base, "prefix-sources", **design)["rows"]
        assert {(r["source"], r["verdict"], r["conversations"]) for r in sources} == {
            ("atlas-calendar", "confirmed", 4), ("birch-calendar", "confirmed", 4),
            ("atlas-calendar", "untested", 1)}
        pairs = get(base, "prefix-contrasts", **design)["rows"]
        assert len({pair["setup"] for pair in pairs}) == 4
        for pair in pairs:
            left, right = pair["examples"]
            assert left["question"].replace("Atlas", "Birch") == right["question"]
            assert (left["source"], right["source"]) == ("atlas-calendar", "birch-calendar")
            assert (left["original"], left["edited"]) == (right["edited"], right["original"])
            for row in pair["examples"]:
                locator = {k:row[k] for k in ("trace", "call", "report")}
                detail = get(base, "prefix-check", **design, **locator, source=row["source"])
                review = detail["review"]
                assert review["verdict"] == "confirmed" and detail["example"]["thread"] == row["thread"]
                index, = review["reversals"]
                edit = review["conditions"][index]
                assert edit["source"] == row["source"] and edit["original"].endswith(row["original"] + " minutes.")
                assert "unavailable" in edit["edit"]["replacement"]
                assert review["runs"][0]["rows"][index]["example"].startswith("Meeting length: " + row["edited"])
                send(base, "/api/sidecar/prefix-check?" + urlencode(dict(**design, **locator, user="other")), expected=404)
        for scope in [{"user":"other"}, {"org":"other"}, {"workflow":"other"}, {"after":"2026-11-21"},
                      {"meta_key":"stage", "meta_value":"absent"}]:
            assert not get(base, "prefix-contrasts", **design, **scope)["rows"]
        for row in sources:
            examples = get(base, "prefix-cases", **design, source=row["source"], verdict=row["verdict"])["rows"]
            assert len(examples) == row["conversations"] and all(r["source"] == row["source"] for r in examples)

        other_thread = trace_for(cases[0], 9)
        span(other_thread)["traceId"] = "b" * 32
        send_traces(base, other_thread)
        updated, = get(base, "prefix-groups", **design)["rows"]
        assert (updated["conversations"], updated["inputs"], updated["confirmed"]) == (9, 8, 9)
    with sqlite3.connect(db) as connection:
        connection.execute("DROP TABLE prefix_checks")
        connection.execute("PRAGMA user_version=26")
    with server(binary, db, "--capture-content") as base:
        assert get(base, "prefix-groups", **design)["rows"] == [updated]
        assert len(get(base, "prefix-contrasts", **design)["rows"]) == 4
        # Matched pairs require comparable runners. A contradictory repeat from a
        # different runner cannot cancel a pair; the same runner must cancel it.
        for i, different_runner in enumerate([True, False]):
            case = deepcopy(cases[0])
            report = case["envelope"]["shadow"]
            first = report["effects"][0]
            first["trials"] = deepcopy(report["baseline"])
            first["summary"] = deepcopy(report["baseline_summary"])
            first["difference_pp"] = 0.0
            if different_runner:
                report["runner_id"] = "different-runner"
            repeat = trace_for(case, 10 + i)
            span(repeat)["traceId"] = f"{100+i:032x}"
            send_traces(base, repeat)
            assert len(get(base, "prefix-contrasts", **design)["rows"]) == (4 if different_runner else 3)
            assert len(get(base, "prefix-cases", verdict="disagreed", **design)["rows"]) == i+1
    with server(binary, folder / "redacted.sqlite") as base:
        send_traces(base, traces[0])
        assert not get(base, "prefix-groups")["rows"]


def measure(binary, folder, cases):
    from sidecar_conflict_coverage import time_reads
    with server(binary, folder / "cost.sqlite", "--capture-content") as base:
        for i in range(128):
            trace = trace_for(cases[i % len(cases)], i + 1)
            span(trace)["traceId"] = f"{i+1:032x}"
            send_traces(base, trace)
        group, = get(base, "prefix-groups")["rows"]
        assert (group["conversations"], group["inputs"]) == (128, 8)
        design = {"design":group["design"]}
        assert len(get(base, "prefix-contrasts", **design)["rows"]) == 4
        for path in ["prefix-groups", "prefix-sources", "prefix-contrasts", "prefix-cases"]:
            time_reads(base, path, {**design, "source":"atlas-calendar"} if path == "prefix-cases" else design)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sdk-python")
    parser.add_argument("--measure", action="store_true")
    args = parser.parse_args()
    cases = json.loads(CASES.read_text())
    with TemporaryDirectory() as work:
        if args.measure:
            measure(args.binary.resolve(), Path(work), cases)
        else:
            check(args.binary.resolve(), Path(work), cases, args.sdk_python)
            print("Verified matched questions, exact evidence, capture gaps, scope, retries and migration.")
