"""Exercise the sidecar API, rank labeled cases, and reproduce a bounded load test."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from copy import deepcopy
from datetime import datetime, timezone
from hashlib import sha256
from itertools import combinations
from math import ceil
import json
import os
from pathlib import Path
import re
import socket
import sqlite3
from statistics import median
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

ROOT = Path(__file__).resolve().parents[1]


def send(base, path, payload=None, expected=200, headers=None):
    if payload is not None:
        data = json.dumps(payload).encode()
        headers = {"Content-Type": "application/json", **(headers or {})}
    else:
        data = None
    try:
        response = urlopen(Request(base + path, data, headers or {}), timeout=20)
    except HTTPError as error:
        response = error
    with response:
        raw = response.read()
        if expected is not None:
            assert response.status == expected, (path, response.status, raw[:300])
        if response.headers.get_content_type() == "application/json":
            return response.status, json.loads(raw)
        return response.status, raw


def send_traces(base, document):
    send(base, "/v1/traces", document)


def sdk_sender(python):
    def export_traces(base, document):
        subprocess.run([python, str(ROOT / "tests/send_sdk_fixture.py"), base + "/v1/traces"],
                       input=json.dumps(document), text=True, check=True)
    return export_traces


def get(base, view, **params):
    return send(base, "/api/sidecar/" + view + "?" + urlencode(params))[1]


def export(*spans):
    return {"resourceSpans": [{"resource": {"attributes": attrs({"service.name": "policy-workflow"})},
                               "scopeSpans": [{"spans": list(spans)}]}]}


def attrs(values):
    return [{"key": key, "value": {"stringValue": value}} for key, value in values.items()]


CLOCK = int(datetime(2026, 10, 12, tzinfo=timezone.utc).timestamp())


def case_span(case, mode, number=0, *, org="heldout", shift=0):
    trace = uuid5(NAMESPACE_URL, f"{org}:{case}:{number}").hex
    span = "1111111111111111"
    text = "Input carrying " + case
    other = text if mode == "duplicate" else "Alternative declaration for " + case
    messages = [{"role": role, "parts": [{"type": "text", "content": value}]}
                for role, value in [("developer", text), ("user", other)]]
    declarations = [{
        "anchor": {"field": "gen_ai.input.messages", "message_index": i, "part_index": 0},
        "source_id": f"{case}-source-{i}",
        "source_version": "v1",
        **({"rule": {"key": case, "scope": "standard", "value": i}} if mode == "conflict" else {}),
    } for i in range(2)]
    output = {
        "traceId": trace, "spanId": span, "name": "private-span-name:" + case,
        "parentSpanId": "2222222222222222",
        "startTimeUnixNano": str((CLOCK + shift + number * 86400) * 10**9),
        "attributes": attrs({
            "gen_ai.operation.name": "chat",
            "gen_ai.request.model": "fixture",
            "gen_ai.input.messages": json.dumps(messages),
            "gen_ai.conversation.id": "thread-" + str(number),
            "nexusweave.context.semantics": json.dumps({"version": 1, "context": declarations}),
            "nexusweave.organization.id": org,
            "nexusweave.user.id": "case-owner",
            "deployment.environment.name": "validation",
            "private.token": "secret-would-be-redacted",
        }),
    }
    return output


def root_span(child):
    return {"traceId": child["traceId"], "spanId": child["parentSpanId"],
            "name": "agent-root", "startTimeUnixNano": str(int(child["startTimeUnixNano"]) - 1000),
            "attributes": []}


@contextmanager
def server(binary, db, *flags):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    storage = ["--demo"] if db is None else ["--db", str(db)]
    with tempfile.TemporaryFile() as logs:
        process = subprocess.Popen(
            [str(binary), "sidecar", *storage, "--listen", f"127.0.0.1:{port}", *flags],
            stdout=logs, stderr=logs,
        )
        try:
            deadline = time.monotonic() + 10
            while True:
                try:
                    get(base, "health")
                    break
                except (URLError, TimeoutError):
                    if process.poll() is not None or time.monotonic() >= deadline:
                        logs.seek(0)
                        raise RuntimeError(logs.read().decode())
                    time.sleep(0.02)
            yield base
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)


def check_capture(binary, folder, producer_python=None):
    db = folder / "redacted.sqlite"
    flags = ("--metadata", "deployment.environment.name")
    child = case_span("redaction", "conflict")
    trace_id = "otlp:" + child["traceId"]
    with server(binary, db, *flags) as base:
        send(base, "/v1/traces", export(child))
        send(base, "/v1/traces", export(child))
        assert not get(base, "findings")["rows"]
        gap, = get(base, "conversations-without-findings")["rows"]
        assert gap["trace"] == trace_id and gap["calls"] == 1 and gap["with_text"] == 0
        assert get(base, "health")["coverage"] == {"traces": 1, "inference_spans": 1, "with_inputs": 0, "with_request": 0, "with_rules": 0}
        rows = get(base, "traces", workflow="policy-workflow", meta_key="deployment.environment.name", meta_value="validation")["rows"]
        assert [row["id"] for row in rows] == [trace_id]
        assert not get(base, "traces", meta_key="private.token", meta_value="secret-would-be-redacted")["rows"]
        send(base, "/v1/traces", export(child), expected=403, headers={"Origin": "https://untrusted.example"})
        if producer_python:
            subprocess.run([producer_python, str(ROOT / "examples/send_otlp_inventory.py"), "--endpoint", base + "/v1/traces",
                            "--conversation-id", "independent-sdk-producer"], check=True)
            assert any(row["thread"] == "independent-sdk-producer" for row in get(base, "threads", org="sdk-demo-org")["rows"])
            assert get(base, "health")["coverage"]["inference_spans"] == 3
    with sqlite3.connect(db) as sql:
        state = "\n".join(value for pair in sql.execute("SELECT raw,run FROM traces") for value in pair)
        assert "secret-would-be-redacted" not in state and "private-span-name" not in state
        assert "Input carrying redaction" not in state and "MUG-42" not in state
    with server(binary, db, *flags) as base:
        send(base, "/v1/traces", export(root_span(child)))
        detail = get(base, "trace", id=trace_id)["trace"]
        assert {span["id"] for span in detail["spans"]} == {child["spanId"], child["parentSpanId"]}
        assert len(detail["calls"]) == 1
    with sqlite3.connect(db) as sql:
        sql.execute("UPDATE traces SET seen=? WHERE id=?", (int(time.time()) - 8 * 86400, trace_id))
    with server(binary, db, *flags) as base:
        assert trace_id not in [row["id"] for row in get(base, "traces")["rows"]]
        assert get(base, "impact", org="heldout")["captured_conversations"] == 0


def check_first_recommendation(binary, folder, python):
    sender = ROOT / "examples/exchange-approval/first_span.py"
    with server(binary, folder / "first-findings.sqlite", "--capture-content") as base:
        env = {**os.environ, "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": base + "/v1/traces"}
        for count, thread in enumerate(("exchange-101", "exchange-102"), start=1):
            subprocess.run([python, str(sender), thread], env=env, check=True)
            finding, = get(base, "findings")["rows"]
            assert (finding["kind"], finding["threads"]) == ("conflict", count)
            assert {s["source"] for s in finding["evidence"]["detail"]["sources"]} == {
                "confirmation-cache", "latest-customer-choice",
            }
            assert get(base, "health")["coverage"]["with_rules"] == count


def check_thread_calls(binary, folder):
    # Deliberately reverse span IDs within one second across two traces.
    spans = []
    for index in range(19):
        span = case_span("page", "conflict", org="thread-test")
        span.update(traceId=f"{1 + index % 2:032x}", spanId=f"{100 - index:016x}",
                    startTimeUnixNano=str(CLOCK * 10**9 + index * 1000))
        span["attributes"] += attrs({"nexusweave.turn.id": f"turn-{index // 2}"})
        spans.append(span)
    tool = spans[17]
    tool["attributes"] = attrs({
        "gen_ai.operation.name": "execute_tool", "gen_ai.tool.name": "update_order",
        "gen_ai.tool.call.id": "order-write", "gen_ai.tool.call.result": "Order write rejected",
        "gen_ai.conversation.id": "thread-0", "nexusweave.organization.id": "thread-test",
        "nexusweave.user.id": "case-owner", "nexusweave.turn.id": "turn-8",
    })
    key = json.dumps(["otlp", "thread-test", "case-owner", "thread-0"], separators=(",", ":"))
    db = folder / "conversation.sqlite"
    flags = ("--capture-content", "--metadata", "deployment.environment.name")
    with server(binary, db, *flags) as base:
        send_traces(base, export(*reversed(spans[1:])))
        send_traces(base, export(spans[0]))
        send_traces(base, export(*spans))
        first = get(base, "thread-calls", thread=key)
        second = get(base, "thread-calls", thread=key, offset=first["next_offset"])
        assert second["offset"] == 15 and second["next_offset"] is None
        rows = first["rows"] + second["rows"]
        assert [row["call"] for row in rows] == [span["spanId"] for span in spans]
        assert rows[0]["dialogue"]["turn"] == "turn-0"
        assert rows[0]["dialogue"]["user"]["text"] == "Alternative declaration for page"
        assert rows[0]["dialogue"]["output_count"] == 0 and rows[0]["findings"][0]["kind"] == "conflict"
        assert rows[17]["dialogue"]["outputs"] == [{
            "kind": "tool_execution", "label": "update_order", "text": "Order write rejected",
        }]
        assert not rows[17]["findings"]
        focus = {"focus_trace": rows[-1]["trace"], "focus_call": rows[-1]["call"]}
        assert get(base, "thread-calls", thread=key, **focus) == second
        for restriction in [{"user": "other"}, {"org": "other"}, {"workflow": "other"},
                            {"meta_key": "deployment.environment.name", "meta_value": "other"},
                            {"after": "2026-10-13"}, {"focus_call": "unknown", "focus_trace": rows[0]["trace"]}]:
            assert get(base, "thread-calls", thread=key, **restriction)["rows"] == []
        for query in ["", "?thread="]:
            assert send(base, "/api/sidecar/thread-calls" + query, expected=None)[0] == 400
    with sqlite3.connect(db) as sql:
        sql.execute("UPDATE conversation_calls SET preview=json_set(preview,'$.outputs',json('[]'),'$.output_count',0) WHERE call_id=?", (tool["spanId"],))
    with server(binary, db, *flags) as base:
        assert get(base, "thread-calls", thread=key, **focus) == second
    with sqlite3.connect(db) as sql:
        sql.execute("DROP TABLE conversation_calls")
        sql.execute("PRAGMA user_version=22")
    with server(binary, db, *flags) as base:
        assert get(base, "thread-calls", thread=key, **focus) == second
    with server(binary, folder / "private-conversation.sqlite") as base:
        send_traces(base, export(spans[0], tool))
        row, tool_row = get(base, "thread-calls", thread=key)["rows"]
        assert "Alternative declaration for page" not in json.dumps(row)
        assert not (row["dialogue"]["user"] or {}).get("text")
        assert "Order write rejected" not in json.dumps(tool_row)


def check_tool_handoffs(binary, db):
    receiver = case_span("handoff", "undeclared")
    start = CLOCK * 10**9
    cases = ["same", "different", "missing", "duplicate", "late", "unknown"]
    receiver["attributes"] = [field for field in receiver["attributes"]
                              if field["key"] not in {"gen_ai.input.messages", "nexusweave.context.semantics"}]
    receiver["attributes"] += attrs({"gen_ai.input.messages": json.dumps([{
        "role": "tool", "parts": [{"type": "tool_call_response", "id": key, "response": "shown"} for key in cases],
    }])})
    executions = []
    for i, key in enumerate(["same", "different", "duplicate", "duplicate", "late", "unknown"], 1):
        attributes = {"gen_ai.operation.name": "execute_tool", "gen_ai.tool.name": "read_record",
                      "gen_ai.tool.call.id": key}
        if key != "unknown":
            attributes["gen_ai.tool.call.result"] = "earlier" if key == "different" else "shown"
        executions.append({"traceId": receiver["traceId"], "spanId": f"{i:016x}",
                           "startTimeUnixNano": str(start - 2000),
                           "endTimeUnixNano": str(start + (1000 if key == "late" else -1000)),
                           "attributes": attrs(attributes)})
    with server(binary, db, "--capture-content") as base:
        send_traces(base, export(receiver, *reversed(executions)))
        details = get(base, "trace", id="otlp:" + receiver["traceId"], call=receiver["spanId"])
        results = {result["invocation"]: result for result in details["received_tool_results"]}
        assert set(results) == set(cases)
        for key, expected in [("same", True), ("different", False), ("unknown", None)]:
            assert results[key]["execution"]
            assert results[key]["tool"] == "read_record"
            assert results[key]["exact_payload_match"] is expected
        for key in ("missing", "duplicate", "late"):
            assert results[key]["execution"] is None and results[key]["exact_payload_match"] is None
        assert [results[key]["context"] for key in cases] == [f"input:0:{i}" for i in range(len(cases))]


def check_conversation_population(binary, db):
    org = "population"
    early = case_span("clear", "undeclared", org=org)
    later = case_span("rules", "conflict", org=org, shift=86400)
    later.update(traceId=early["traceId"], spanId="3333333333333333")
    quiet = case_span("second", "undeclared", org=org, number=1)
    flags = ("--capture-content", "--metadata", "deployment.environment.name")
    with server(binary, db, *flags) as base:
        send_traces(base, export(early, quiet))
        send_traces(base, export(later))
        send_traces(base, export(early, quiet, later))
        def counts(**filters):
            impact = get(base, "impact", org=org, **filters)
            summary = get(base, "summary", org=org, **filters)
            assert summary.pop("repeat_groups") == 0
            assert summary == {key: impact[key] for key in ("conversations", "captured_conversations", "recommendations", "factors")}
            return tuple(impact[key] for key in ("captured_conversations", "conversations", "without_findings"))
        assert counts() == (2, 1, 1)
        assert counts(before="2026-10-13") == (1, 0, 1)
        assert counts(after="2026-10-13", before="2026-10-14") == (2, 1, 1)
        assert counts(after="2026-10-14") == (0, 0, 0)
        assert counts(user="other") == counts(meta_key="deployment.environment.name", meta_value="other") == (0, 0, 0)
        assert counts(trace="otlp:" + early["traceId"], call=early["spanId"]) == (1, 0, 1)
        page = get(base, "conversations-without-findings", org=org)
        row, = page["rows"]
        assert row["trace"] == "otlp:" + quiet["traceId"] and row["call"] == quiet["spanId"]
        assert row["calls"] == row["with_text"] == 1
        first, = get(base, "conversations-without-findings", org=org, before="2026-10-13")["rows"]
        assert first["trace"] == "otlp:" + early["traceId"] and first["call"] == early["spanId"]
    with sqlite3.connect(db) as sql:
        sql.execute("DROP TABLE captured_calls")
        sql.execute("PRAGMA user_version=16")
    with server(binary, db, *flags) as base:
        assert get(base, "conversations-without-findings", org=org) == page
        assert get(base, "impact", org=org)["captured_conversations"] == 2


def check_screen_suggestions(binary, folder):
    fixtures = ROOT / "examples/context-screening"
    request = json.loads((fixtures / "request.json").read_text())
    screen = json.loads((fixtures / "evidence.json").read_text())[0]["calls"][0]["context_screening"]["report"]
    # Reuse the two directional scores saved for each pair, with the same two texts and roles.
    cases = []
    for index, kind in [(2, "possible_overlap"), (3, "possible_conflict")]:
        messages = request["attributes"]["gen_ai.input.messages"]
        selected = [messages[0], messages[index]]
        pair = next(p for p in screen["pairs"] if p["sources"] == [0, index])
        report = {**screen, "total_pairs": 1, "pairs": [{**pair, "sources": [0, 1]}],
                  "sources": [screen["sources"][0], {**screen["sources"][index], "context_id": "input:1:0"}]}
        child = case_span(kind, "undeclared", number=index)
        child["attributes"] = attrs({"gen_ai.operation.name":"chat", "gen_ai.request.model":"support-demo",
            "gen_ai.conversation.id":kind, "gen_ai.input.messages":json.dumps(selected),
            "nexusweave.context_screening":json.dumps(report)})
        cases.append((child, selected, report, kind))
    for capture in (False, True):
        with server(binary, folder / f"screen-{capture}.sqlite", *(["--capture-content"] if capture else [])) as base:
            for child, messages, report, kind in cases:
                send_traces(base, export(child))
                row, = get(base, "conversations-without-findings", trace="otlp:" + child["traceId"])["rows"]
                assert bool(row["screen"]) == capture
                if capture:
                    assert row["screen"]["suggestion"] == kind and row["screen"]["flagged_pairs"] == 1
                    assert [s["text"] for s in row["screen"]["sources"]] == [m["parts"][0]["content"] for m in messages]
                pending = {"version":1, "attributes":{"gen_ai.input.messages":messages}}
                envelope = {"request":pending, "screening":report}
                preview = send(base, "/api/sidecar/preview", envelope)[1]
                assert preview["screen_suggestion"]["suggestion"] == kind and preview["total"] == 0
                stale = deepcopy(envelope)
                stale["screening"]["sources"][0]["text_sha256"] = "0" * 64
                rejected = send(base, "/api/sidecar/preview", stale)[1]
                assert rejected["screen_suggestion"] is None and rejected["unverified_probes"]
            population = get(base, "impact")
            assert population["without_findings"] == 2
            assert population["without_findings_with_screen_suggestion"] == (2 if capture else 0)

            groups = get(base, "screened-pairs")["rows"]
            assert len(groups) == (2 if capture else 0)
            if capture:
                assert all(g["conversations"] == g["calls"] == 1 and g["tested"] == 0 for g in groups)
                # Reverse the same pair without changing either source role or scorer.
                _, texts, report, relation = cases[-1]
                reversed_report = deepcopy(report)
                reversed_report["sources"] = [{**src, "context_id":f"input:{i}:0"} for i, src in enumerate(reversed(report["sources"]))]
                pair = reversed_report["pairs"][0]
                pair["left_to_right"], pair["right_to_left"] = pair["right_to_left"], pair["left_to_right"]
                def reversal(name, scores):
                    span = case_span(name, "undeclared")
                    span["attributes"] = attrs({"gen_ai.operation.name":"chat","gen_ai.conversation.id":name,
                        "gen_ai.input.messages":json.dumps(texts[::-1]),"nexusweave.context_screening":json.dumps(scores)})
                    send_traces(base, export(span))
                reversal("reversed", reversed_report)
                pattern, = (g for g in groups if g["relation"] == relation)
                members = get(base, "screened-pair-examples", pattern=pattern["pattern"], relation=relation)["rows"]
                assert len(members) == 2 and sorted(r["thread"] for r in members) == [relation, "reversed"]
                assert all(len(r["sources"]) == 2 for r in members)
                different = deepcopy(reversed_report)
                different["scorer"]["revision"] += "-test"
                reversal("different-scorer", different)
                assert len(get(base, "screened-pairs")["rows"]) == 3
                multi = case_span("all-screened", "undeclared")
                multi["attributes"] = attrs({"gen_ai.operation.name":"chat", "gen_ai.conversation.id":"multi",
                    "gen_ai.input.messages":json.dumps(request["attributes"]["gen_ai.input.messages"]),
                    "nexusweave.context_screening":json.dumps(screen)})
                send_traces(base, export(multi))
                scoped = {"trace":"otlp:" + multi["traceId"]}
                assert get(base, "findings", **scoped)["rows"][0]["kind"] == "duplicate"
                groups = get(base, "screened-pairs", **scoped)["rows"]
                occurrences = [row for group in groups for row in get(base, "screened-pair-examples",
                    pattern=group["pattern"], relation=group["relation"], **scoped)["rows"]]
                expected = [[screen["sources"][i]["context_id"] for i in pair["sources"]] for pair in screen["pairs"]
                            if pair.get("suggestion") in ("possible_conflict","possible_overlap")]
                assert sorted([[s["id"] for s in row["sources"]] for row in occurrences]) == sorted(expected)
                assert not get(base, "screened-pairs", after="2026-11-01", **scoped)["rows"]
                assert not get(base, "screened-pairs", call="absent", **scoped)["rows"]

    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare
    with server(binary, folder / "one-arm.sqlite", "--capture-content") as base:
        trace, measured = prepare("one-arm", followup=True)
        send_traces(base, trace)
        # Keep only the actual FAQ arm, which preserved the answer in this case.
        measured["variants"] = measured["variants"][1:]
        measured["pair"] = None
        measured["id"] += "-faq-only"
        send(base, "/api/replay-reports", measured, expected=201)
        group, = get(base, "screened-pairs")["rows"]
        policy, faq = group["sources"]
        assert policy["digest"] not in group["source_outcomes"]
        assert group["source_outcomes"][faq["digest"]] == {"changed":0,"held":1}
    with server(binary, folder / "followup.sqlite", "--capture-content") as base:
        trace, report = prepare("journal-return-followup", followup=True)
        send_traces(base, trace)
        gap, = get(base, "conversations-without-findings")["rows"]
        assert [s["source"] for s in gap["screen"]["sources"]] == ["journal-policy", "journal-faq"]
        group, = get(base, "screened-pairs")["rows"]
        query = {"pattern":group["pattern"], "relation":group["relation"]}
        assert group["calls"] == group["conversations"] == 1 and group["tested"] == 0
        send(base, "/api/replay-reports", report, expected=201)
        send(base, "/api/replay-reports", report)
        impact = get(base, "impact")
        assert impact["captured_conversations"] == impact["evidence_counts"]["proposal"] == 1
        assert impact["without_findings"] == 0
        edits = get(base, "outcome-factors", level=1, by="edit")["rows"]
        assert [row["sources"][0]["source"] for row in edits if len(row["sources"]) == 1] == ["journal-policy"]
        earlier, earlier_report = prepare("earlier")
        send_traces(base, earlier)
        groups = get(base, "screened-pairs")["rows"]
        assert len(groups) == 1 and groups[0]["tested"] == 1 and groups[0]["conversations"] == 2
        send(base, "/api/replay-reports", earlier_report, expected=201)
        send_traces(base, trace)
        group, = get(base, "screened-pairs")["rows"]
        assert group["conversations"] == group["calls"] == group["tested"] == 2
        assert group["source_outcomes"] == {s["digest"]:{"changed":1, "held":1} for s in group["sources"]}
        assert get(base, "screened-pairs", **query)["rows"] == [group]
        members = get(base, "screened-pair-examples", **query)
        changed = {row["thread"]:[t["context"] for t in row["comparisons"] if t["pair"]["changed"]]
                   for row in members["rows"]}
        assert changed == {"journal-return-followup":["input:0:0"], "earlier":["input:1:0"]}
        south, = get(base, "screened-pair-examples", org="journal-south", user="support-41", **query)["rows"]
        assert south["call"] == report["location"]["call_id"] and len(south["comparisons"]) == 2
        for test in south["comparisons"]:
            exact, = get(base, "findings", group=test["group"], trace=south["trace"], call=south["call"],
                         replay=test["replay"], decision=test["decision"], context=test["context"])["rows"]
            assert exact["evidence"]["detail"]["response_pair"] == test["pair"]
        assert not get(base, "screened-pair-examples", **{**query, "relation":"possible_overlap"})["rows"]
        from send_otlp_replay import value_report
        for prior, followup in [(earlier_report,False),(report,True)]:
            send(base, "/api/replay-reports", value_report(prior, followup=followup), expected=201)
        assert get(base, "screened-pairs")["rows"] == [group]
        members = get(base, "screened-pair-examples", **query)
        for row in members["rows"]:
            assert len(row["comparisons"]) == 4 and row["last_user_text"]["text"]
            values = [t for t in row["comparisons"] if "45 days" in t["replacement"]["text"]]
            assert len(values) == 2
            edited = {t["context"]: t["pair"]["edited"]["outcome_preview"] for t in values}
            assert edited == ({"input:0:0":"120","input:1:0":"30"} if row["thread"] == "journal-return-followup"
                              else {"input:0:0":"60","input:1:0":"45"})
    with sqlite3.connect(folder / "followup.sqlite") as sql:
        sql.execute("DROP TABLE screened_pairs")
        sql.execute("DROP TABLE screened_inputs")
        sql.execute("PRAGMA user_version=17")
    with server(binary, folder / "followup.sqlite", "--capture-content") as base:
        assert get(base, "screened-pair-examples", **query) == members


def check_declared_conflict_replays(binary, folder):
    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare, value_report
    with server(binary, folder / "declared-windows.sqlite", "--capture-content") as base:
        first, report = prepare("tested-journal", declare_windows=True)
        other, other_report = prepare("untested-journal", declare_windows=True)
        send_traces(base, first)
        send_traces(base, other)
        card, = get(base, "findings", kind="conflict")["rows"]
        assert "conflict_test" not in card
        group = card["id"]

        def read(location):
            row, = get(base, "findings", group=group, trace=location["run_id"], call=location["call_id"])["rows"]
            trial = row["conflict_test"]
            outcomes = {e["changed"] for source in trial["sources"] for e in source["comparisons"]}
            title = ("Repeat the edit that changed the proposal" if True in outcomes else
                     "Review which edits preserved the proposal" if outcomes else "Resolve the competing rules")
            assert row["action"]["title"] == title
            return trial

        assert all(not s["comparisons"] for s in read(report["location"])["sources"])
        for trial in (report, value_report(report, followup=False)):
            send(base, "/api/replay-reports", trial, expected=201)
        result = read(report["location"])
        assert result["recorded_request"] and len(result["decisions"]) == 1
        for source in result["sources"]:
            assert source["captured"] and source["matching_fields"] == 1
            example, = source["comparisons"]
            assert example["reports"] == 2 and example["changed"] == (source["id"] == "input:1:0")
            target, = get(base, "findings", group=example["group"], trace=report["location"]["run_id"],
                          call=report["location"]["call_id"], replay=example["replay"],
                          decision=example["decision"], context=source["id"])["rows"]
            assert example["pair"] == target["evidence"]["detail"]["response_pair"]
        assert all(not s["comparisons"] for s in read(other_report["location"])["sources"])
        # Submit only the measured policy arm for the other request and omit its multi-source test.
        other_report.pop("pair")
        other_report["variants"] = [v for v in other_report["variants"] if v["intervention"]["context_id"] == "input:0:0"]
        send(base, "/api/replay-reports", other_report, expected=201)
        held = read(other_report["location"])
        assert [e["changed"] for source in held["sources"] for e in source["comparisons"]] == [False]
        assert not get(base, "findings", group=group, user="unrelated")["rows"]


def check_replay(base, send_trace=send_traces):
    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare as prepare_replay

    trace, report = prepare_replay("held-out-experiment")
    path = "/api/replay-reports"
    send_trace(base, trace)
    uncovered, = get(base, "conversations-without-findings", trace=report["location"]["run_id"])["rows"]
    assert uncovered["screen"]["suggestion"] == "possible_conflict"
    tampered = deepcopy(report)
    tampered["baseline_request_sha256"] = "0" * 64
    send(base, path, tampered, expected=400)
    send(base, path, report, expected=201)
    send(base, path, report)
    assert not get(base, "conversations-without-findings", trace=report["location"]["run_id"])["rows"]
    incomplete = deepcopy(report)
    incomplete["id"] += "-incomplete"
    variant = incomplete["variants"][0]
    variant["trials"] = [{"status": "failed"}]
    variant["summary"] = {"attempted": 1, "completed": 0, "matched": 0}
    variant["difference_pp"] = None
    incomplete.pop("pair", None)
    send(base, path, incomplete, expected=201)
    location = report["location"]
    details = get(base, "trace", id=location["run_id"])
    assert details["replay_reports"] == 2
    measured = [item for item in details["findings"] if item["detail"]["measurement"]["replay_id"] == report["id"]]
    assert sorted(item["detail"]["effect_pp"] for item in measured) == [-100.0, -100.0, 0.0]
    for item in measured:
        detail = item["detail"]
        if detail["edit"] == "individual":
            arm = next(v for v in report["variants"] if v["intervention"]["context_id"] == detail["sources"][0]["id"])
            assert detail["baseline_answer"] == report["baseline"][0]["choice"]["outcome"]
            assert detail["edited_answer"] == arm["trials"][0]["choice"]["outcome"]
    assert all(item["call_id"] == location["call_id"] for item in measured)
    assert all(len(item["detail"]["measurement"]["location"]) >= 3 for item in measured)
    assert not any(item["detail"]["measurement"]["replay_id"] == incomplete["id"] and item["detail"]["effect_pp"] == 0
                   for item in details["findings"])
    return len(measured)


def check_replacement_comparison(base):
    # These edited trial records exercise the validator and comparison logic.
    from send_otlp_replay import prepare
    trace, original = prepare("replacement-validation")
    original.pop("pair")
    original["variants"] = [original["variants"][1]]
    send_traces(base, trace)
    send(base, "/api/replay-reports", original, expected=201)

    second = deepcopy(original)
    second["id"] = "second-wording"
    arm = second["variants"][0]
    arm["intervention"]["replacement"] = "Another synthetic replacement"
    arm.update(trials=second["baseline"], summary=second["baseline_summary"], difference_pp=0.0)
    for name, changes in [("other-runner", {"runner_id": "another-runner"}),
                          ("different-rule", {"match_rule": {"kind": "text_capture", "pattern": "(.*)"}})]:
        incompatible = deepcopy(second)
        incompatible.update(id=name, **changes)
        send(base, "/api/replay-reports", incompatible, expected=201)
    assert not get(base, "findings", kind="replacement")["rows"]
    send(base, "/api/replay-reports", second, expected=201)
    send(base, "/api/replay-reports", second)
    card, = get(base, "findings", kind="replacement")["rows"]
    detail = card["evidence"]["detail"]
    assert card["occurrences"] == 1 and detail["wordings"] == detail["consistent_wordings"] == 2
    assert {c["report_id"] for c in detail["conditions"]} == {original["id"], second["id"]}
    assert {c["edits"][0]["matched"] for c in detail["conditions"]} == {0, 1}
    assert not get(base, "findings", kind="replacement", user="absent")["rows"]
    # Repeats of the same wording now disagree. The two-wording comparison must retire.
    conflicting = deepcopy(original)
    conflicting["id"] = "second-wording-again"
    conflicting["variants"][0]["intervention"] = arm["intervention"]
    send(base, "/api/replay-reports", conflicting, expected=201)
    assert not get(base, "findings", kind="replacement")["rows"]


def check_replay_representative(base):
    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare as prepare_replay

    changed_trace, changed_report = prepare_replay("older-measured-change")
    equal_trace, equal_report = prepare_replay("newer-measured-equality")
    equal_span = equal_trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
    for field in ["startTimeUnixNano", "endTimeUnixNano"]:
        equal_span[field] = str(int(equal_span[field]) + 2 * 10**9)
    equal_report.pop("pair", None)
    equal_arm = equal_report["variants"][1]
    equal_arm["trials"] = deepcopy(equal_report["baseline"])
    equal_arm["summary"] = deepcopy(equal_report["baseline_summary"])
    equal_arm["difference_pp"] = 0.0
    for trace, report in [(changed_trace, changed_report), (equal_trace, equal_report)]:
        send_traces(base, trace)
        send(base, "/api/replay-reports", report, expected=201)
    rows = get(base, "findings", kind="replay")["rows"]
    card = next(card for card in rows if card["evidence"]["detail"]["edit"] == "individual" and card["priority"] == 1)
    assert card["traces"] == 2 and card["latest"] > card["evidence"]["observed"]
    assert card["evidence"]["trace_id"] == changed_report["location"]["run_id"]
    assert card["evidence"]["detail"]["effect_pp"] == -100.0
    selected = {"group": card["id"], "trace": equal_report["location"]["run_id"],
                "call": equal_report["location"]["call_id"], "replay": equal_report["id"],
                "context": card["evidence"]["detail"]["measurement"]["context_id"]}
    specific, = get(base, "findings", **selected)["rows"]
    assert specific["evidence"]["trace_id"] == selected["trace"] and specific["evidence"]["detail"]["effect_pp"] == 0
    assert len(get(base, "occurrences", **selected)["rows"]) == 1
    assert not get(base, "findings", **{**selected, "context": "absent"})["rows"]
    counts = get(base, "impact")["evidence_counts"]
    assert counts == {"proposal": 1, "rate": 0, "score": 0, "tested": 1, "inputs": 0}, counts
    equal, = get(base, "conversation-outcomes", level=4)["rows"]
    assert equal["unchanged_pair"] and equal["trace"] == equal_report["location"]["run_id"]
    outcomes = get(base, "impact")["replay_outcomes"]
    assert outcomes == {"mixed": 1, "changed_only": 0, "equal_only": 1, "unmeasured": 0}, outcomes
    outcomes = get(base, "impact", factor=card["evidence"]["detail"]["factor"])["replay_outcomes"]
    assert outcomes == {"mixed": 0, "changed_only": 1, "equal_only": 1, "unmeasured": 0}, outcomes

    proposal_factors = get(base, "outcome-factors", level=1)["rows"]
    assert len(proposal_factors) == 2 and all(r["conversations"] == r["cohort_size"] == 1 for r in proposal_factors)
    for factor in proposal_factors:
        row, = get(base, "conversation-outcomes", level=1, cohort_factor=factor["factor"])["rows"]
        assert row["factor"] == factor["factor"] and row["trace"] == changed_report["location"]["run_id"]
        assert row["decision"] == changed_report["location"]["decision_id"]
    # One single edit was ineffective in a conversation where another changed the proposal.
    # A filter applied before classification would incorrectly move that conversation to level 4.
    weaker = next(r for r in rows if r["evidence"]["detail"]["edit"] == "individual"
                  and r["evidence"]["detail"]["effect_pp"] == 0)
    factor_id = weaker["evidence"]["detail"]["factor"]
    reclassified = get(base, "conversation-outcomes", level=4, factor=factor_id)["rows"]
    assert {r["thread"] for r in reclassified} == {"older-measured-change","newer-measured-equality"}
    preserved, = get(base, "conversation-outcomes", level=4, cohort_factor=factor_id)["rows"]
    assert preserved["thread"] == "newer-measured-equality"
    group = next(r for r in get(base, "outcome-factors", level=4)["rows"] if r["factor"] == factor_id)
    assert group["conversations"] == group["cohort_size"] == 1
    send(base, "/api/sidecar/outcome-factors", expected=400)
    send(base, "/api/sidecar/outcome-factors?level=6", expected=400)
    send(base, "/api/sidecar/conversation-outcomes?cohort_factor=unknown", expected=400)

    # Mixed proposals in one arm can shift the rate without establishing a uniform answer pair.
    mixed_trace, mixed = prepare_replay("mixed-proposal-trials")
    mixed.pop("pair")
    mixed["variants"] = [mixed["variants"][1]]
    changed = mixed["variants"][0]
    changed["trials"] += deepcopy(mixed["baseline"])
    mixed["baseline"] *= 2
    mixed["baseline_summary"] = {"attempted": 2, "completed": 2, "matched": 2}
    changed.update(summary={"attempted": 2, "completed": 2, "matched": 1}, difference_pp=-50.0)
    send_traces(base, mixed_trace)
    send(base, "/api/replay-reports", mixed, expected=201)
    rate, = get(base, "conversation-outcomes", level=2)["rows"]
    assert rate["thread"] == "mixed-proposal-trials" and rate["replay"] == mixed["id"]
    assert get(base, "impact", trace=rate["trace"])["evidence_counts"]["rate"] == 1


def check_cohort_paging(base):
    scope = {"org":"cohort-pages", "level":5}
    # Sixteen groups in one conversation, then sixteen conversations sharing the first group.
    traces = [case_span("field-" + str(i), "conflict", org=scope["org"]) for i in range(16)]
    traces.extend(case_span("field-0", "conflict", number=i, org=scope["org"]) for i in range(1,16))
    sibling_call = deepcopy(traces[0])
    sibling_call["spanId"] = "3333333333333333"
    traces.append(sibling_call)
    send_traces(base, export(*traces))
    send_traces(base, export(*traces))
    first = get(base, "outcome-factors", **scope)
    second = get(base, "outcome-factors", **scope, offset=first["next_offset"])
    assert len(first["rows"]) == 15 and len(second["rows"]) == 1 and second["next_offset"] is None
    unseen = get(base, "proposal-conversations", result="no_pair", org=scope["org"])
    last = get(base, "proposal-conversations", result="no_pair", org=scope["org"], offset=unseen["next_offset"])
    assert len(unseen["rows"]) == 15 and len(last["rows"]) == 1 and last["next_offset"] is None
    assert len({r["thread"] for r in unseen["rows"] + last["rows"]}) == 16
    rows = first["rows"] + second["rows"]
    assert len({r["factor"] for r in rows}) == 16
    assert {r["cohort_size"] for r in rows} == {16}
    top = rows[0]
    assert (top["conversations"], top["requests"]) == (16, 17)
    assert all(r["conversations"] == 1 for r in rows[1:])
    first = get(base, "conversation-outcomes", **scope, cohort_factor=top["factor"])
    last = get(base, "conversation-outcomes", **scope, cohort_factor=top["factor"], offset=first["next_offset"])
    assert len({r["thread"] for r in first["rows"]+last["rows"]}) == 16 and last["next_offset"] is None
    day = get(base, "outcome-factors", **scope, after="2026-10-13", before="2026-10-14")
    assert len(day["rows"]) == 1 and day["rows"][0]["cohort_size"] == 1
    assert not get(base, "outcome-factors", **scope, user="missing")["rows"]


def check_joint_proposals(base):
    # Synthetic trials deliberately pass a coarse rule even when their complete answers differ.
    from send_otlp_replay import prepare

    def record(thread, outcomes, samples=1, owner="first"):
        trace, report = prepare(thread)
        span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        fields = {attr["key"]: attr["value"] for attr in span["attributes"]}
        fields["nexusweave.organization.id"]["stringValue"] = "joint-check"
        fields["nexusweave.user.id"]["stringValue"] = owner
        report["match_rule"] = {"kind": "kind_and_label"}
        baseline = report["baseline"][0]
        report["baseline"] *= samples
        summary = {"attempted": samples, "completed": samples, "matched": samples}
        report["baseline_summary"] = summary
        for arm, values in zip([*report["variants"], report["pair"]], outcomes, strict=True):
            trials = [deepcopy(baseline) for _ in range(samples)]
            for trial, value in zip(trials, values, strict=True):
                trial["choice"]["outcome"] = value
            arm.update(trials=trials, summary=summary, difference_pp=0)
        report["pair"]["interaction_pp"] = 0
        send_traces(base, trace)
        send(base, "/api/replay-reports", report, expected=201)
        return report

    scope = {"org": "joint-check", "kind": "replay"}
    held, changed = "60", "different answer"
    first = record("same-thread", [[held], [held], [changed]])
    def joint(**filters):
        cards = get(base, "findings", **scope, **filters)["rows"]
        return next(c for c in cards if c["evidence"]["detail"]["edit"] == "joint")
    card = joint()
    group = card["id"]
    assert card["evidence"]["detail"]["effect_pp"] == 0 and card["evidence"]["detail"]["joint_pattern"] == "joint_only"
    assert get(base, "factors", **scope)["rows"][0]["signal"] == "joint_proposal"
    assert get(base, "impact", group=group)["changed"] == 1
    assert get(base, "impact", group=group)["replay_outcomes"]["equal_only"] == 1
    record("same-thread", [[changed], [held], [changed]])
    record("another-thread", [[changed], [held], [held]], owner="second")
    edit_scope = dict(org="joint-check", level=1, by="edit")
    only_single, = get(base, "outcome-factors", user="second", **edit_scope)["rows"]
    assert len(only_single["sources"]) == 1
    joint_scope = dict(user="second", group=group, **edit_scope)
    assert get(base, "outcome-factors", **joint_scope)["rows"] == [only_single]
    one, = get(base, "conversation-outcomes", **joint_scope, cohort_factor=only_single["factor"])["rows"]
    assert one["witness"]["arm"] == only_single["sources"][0]["id"]
    only_joint, = get(base, "outcome-factors", trace=first["location"]["run_id"], **edit_scope)["rows"]
    assert len(only_joint["sources"]) == 2
    assert not get(base, "conversation-outcomes", user="second", cohort_factor=only_joint["factor"], **edit_scope)["rows"]
    record("same-thread", [[held], [held], [held]])
    uncertain = record("mixed", [[held, changed], [held, held], [changed, changed]], samples=2)
    rows = joint(group=group)["joint_patterns"]
    assert {r["pattern"] for r in rows} == {"joint_only","single_and_joint","single_only","unchanged"}
    assert all(r["inputs"] == r["conversations"] == r["observations"] == 1 for r in rows)
    assert len(joint(group=group,user="first")["joint_patterns"]) == 3
    for row in rows:
        subset = get(base, "occurrences", **scope, group=group, joint_pattern=row["pattern"], baseline_trials=1)["rows"]
        occurrence, = subset
        m = occurrence["detail"]["measurement"]
        picked = joint(group=group, trace=occurrence["trace"], call=occurrence["call"],
                       decision=m["location"]["decision_id"], replay=m["replay_id"])
        assert picked["evidence"]["detail"]["joint_pattern"] == row["pattern"]
        impact = get(base, "impact", group=group, trace=occurrence["trace"])
        assert impact["changed"] == impact["evidence_counts"]["proposal"] == int(row["pattern"] != "unchanged")
    mixed = joint(group=group,trace=uncertain["location"]["run_id"])
    assert mixed["joint_patterns"][0]["pattern"] == "unresolved"
    assert mixed["evidence"]["detail"]["single_edits"][0]["proposal"] is None
    assert not get(base, "findings", group=group, decision="elsewhere")["rows"]
    send(base, "/api/replay-reports", first)
    assert joint(group=group)["joint_patterns"] == rows
    return group, rows


def check_replay_variation(base):
    # These synthetic trial outcomes exercise grouping, not model performance.
    from send_otlp_replay import prepare

    def record(question, shifted, *, reorder_keys=False, samples=1):
        trace, report = prepare("variation-fixture")
        span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        fields = {row["key"]: row["value"] for row in span["attributes"]}
        fields["nexusweave.organization.id"]["stringValue"] = "variation-fixture"
        raw = fields["nexusweave.request.body"]
        body = json.loads(raw["stringValue"])
        body["messages"][-1]["content"] = question
        if reorder_keys:
            body = dict(reversed(list(body.items())))
        raw["stringValue"] = json.dumps(body, separators=(",", ":"))
        report["baseline_request_sha256"] = sha256(raw["stringValue"].encode()).hexdigest()
        messages = json.loads(fields["gen_ai.input.messages"]["stringValue"])
        messages[-1]["parts"][0]["content"] = question
        fields["gen_ai.input.messages"]["stringValue"] = json.dumps(messages)
        report.pop("pair", None)
        arm = report["variants"][1]
        report["variants"] = [arm]
        if not shifted:
            arm["trials"] = deepcopy(report["baseline"])
            arm["summary"] = deepcopy(report["baseline_summary"])
            arm["difference_pp"] = 0
        for trials, summary in [(report["baseline"], report["baseline_summary"]), (arm["trials"], arm["summary"])]:
            trials *= samples
            for key in ("attempted", "completed", "matched"):
                summary[key] *= samples
        send_traces(base, trace)
        send(base, "/api/replay-reports", report, expected=201)
        return report

    def patterns(**scope):
        return get(base, "replay-variations", org="variation-fixture", **scope)["rows"]

    changed = record("Question A", True)
    record("Question A", True, reorder_keys=True)
    record("Question B", False, samples=2)
    assert not patterns(), "Different trial counts cannot form a comparison."
    equal = record("Question B", False)
    pattern, = patterns()
    assert (pattern["changed_inputs"], pattern["equal_inputs"], pattern["mixed_inputs"]) == (1, 1, 0)
    assert not pattern["matched_fragment_order"]
    assert pattern["equal_example"]["measurement"]["location"] == equal["location"]
    assert pattern["changed_example"]["measurement"]["replay_id"] == changed["id"]
    assert pattern["changed_example"]["effect_pp"] != 0 and pattern["equal_example"]["effect_pp"] == 0
    assert get(base, "factors", org="variation-fixture")["rows"][0]["signal"] == "variation"
    assert not patterns(user="absent") and not patterns(trace=equal["location"]["run_id"])
    record("Question A", False)
    assert not patterns(), "Inconsistent repeats cannot supply either representative input."
    record("Question C", True)
    pattern, = patterns()
    assert (pattern["changed_inputs"], pattern["equal_inputs"], pattern["mixed_inputs"]) == (1, 1, 1)


def bound_order_reports(base):
    fixture = json.loads((ROOT / "examples/redundant-routing/measured/local-order.json").read_text())
    for run in fixture["runs"]:
        call, = run["calls"]
        span = case_span(run["id"], "order")
        request = json.loads(call["recorded_request"]["body"])
        declarations = [{key: item[key] for key in ("id", "source_id", "source_version", "role", "request_fragment")}
                        for item in call["context"]]
        span["attributes"] = attrs({
            "gen_ai.operation.name": "chat", "gen_ai.request.model": call["model"],
            "gen_ai.conversation.id": run["conversation_id"], "nexusweave.organization.id": "order-check",
            "nexusweave.user.id": call["context"][0]["source_id"],
            "nexusweave.request.body": call["recorded_request"]["body"],
            "nexusweave.context.semantics": json.dumps({"version": 1, "request_sources": declarations}),
            "gen_ai.input.messages": json.dumps([{"role": m["role"], "parts": [{"type": "text", "content": m["content"]}]}
                                                 for m in request["messages"]]),
            "gen_ai.output.messages": json.dumps([{"role": "assistant", "parts": [
                {"type": "text", "content": call["decisions"][0]["outcome"]}]}]),
        })
        send_traces(base, export(span))
        for report in fixture["reports"]:
            if report["location"]["run_id"] != run["id"]:
                continue
            report["location"] = {"run_id": "otlp:" + span["traceId"], "call_id": span["spanId"], "decision_id": "output:0:0"}
            for arm in report["variants"]:
                arm["intervention"]["context_id"] = "request-source:" + arm["intervention"]["context_id"]
            yield call["recorded_request"]["body"], report


def check_order_tests(base):
    bodies, reports = zip(*bound_order_reports(base))
    for report in reports:
        send(base, "/api/replay-reports", report, expected=201)
    pairs = get(base, "order-tests", org="order-check")["rows"]
    assert [pair["differs"] for pair in pairs] == [1, 1, 0, 0], pairs
    assert all(pair["total"] == 4 and pair["differing"] == 2 for pair in pairs)
    assert not get(base, "order-tests", org="order-check", user="routing-note")["rows"]
    assert get(base, "factors", org="order-check")["rows"][0]["signal"] == "order"
    pending = json.loads((ROOT / "examples/redundant-routing/pending-maple.json").read_text())
    attributes = pending["attributes"]
    attributes.pop("nexusweave.history.source")
    attributes.update({"nexusweave.organization.id": "order-check", "nexusweave.user.id": "dispatch-guide"})
    def prior(request):
        return send(base, "/api/sidecar/preview", {"request": request})[1]["history"]["edits"]
    coverage = get(base, "health")["coverage"]
    top = prior(pending)[0]
    assert (top["example"]["source"], top["requests"], top["lower"], top["mixed"]) == ("routing-note", 2, 2, 0)
    for key in ["nexusweave.organization.id", "nexusweave.user.id", "nexusweave.history.source", "gen_ai.request.model"]:
        other = deepcopy(pending)
        other["attributes"][key] = "different"
        assert prior(other) == []
    repeated = deepcopy(pending)
    repeated["attributes"]["nexusweave.request.body"] = bodies[0]
    assert not prior(repeated), "An identical prior input must not count toward transfer."
    for change in ["version", "system", "temperature"]:
        other = deepcopy(pending)
        fields = other["attributes"]
        body = json.loads(fields["nexusweave.request.body"])
        if change == "version":
            fields["nexusweave.context.semantics"]["request_sources"][0]["source_version"] = "v2"
        elif change == "system":
            body["messages"][0]["content"] += " Changed instructions."
        else:
            body["temperature"] = 0.9
        fields["nexusweave.request.body"] = json.dumps(body)
        assert prior(other) == []
    assert get(base, "health")["coverage"] == coverage
    # An inconsistent repeat in one orientation retires only that matched comparison.
    repeat = deepcopy(reports[0])
    repeat["id"] += "-repeat"
    changed = next(arm for arm in repeat["variants"] if arm["difference_pp"])
    changed.update(trials=repeat["baseline"], summary=repeat["baseline_summary"], difference_pp=0)
    repeat["pair"]["interaction_pp"] = -100
    send(base, "/api/replay-reports", repeat, expected=201)
    assert len(get(base, "order-tests", org="order-check")["rows"]) == 3
    revised = next(edit for edit in prior(pending) if edit["example"]["source"] == "routing-note"
                   and edit["example"]["replacement"] == top["example"]["replacement"])
    assert revised["requests"] == 2 and revised["mixed"] == revised["lower"] == 1


def check_history_proposals(base):
    # All of these proposals satisfy the coarse scoring rule, including the changed answers.
    saved = [r for _, r in bound_order_reports(base) if r["id"].endswith("dispatch-guide-1")]
    assert len(saved) == 2
    pending = json.loads((ROOT / "examples/redundant-routing/pending-maple.json").read_text())
    pending["attributes"].update({"nexusweave.history.source":"otlp", "nexusweave.organization.id":"order-check",
                                  "nexusweave.user.id":"dispatch-guide"})
    def replay(original, suffix, outcomes):
        report = deepcopy(original)
        report["id"] += suffix
        report["match_rule"] = {"kind":"kind_and_label"}
        report["baseline"].append(deepcopy(report["baseline"][0]))
        summary = {"attempted":2,"completed":2,"matched":2}
        report["baseline_summary"] = summary
        for arm in [*report["variants"],report["pair"]]:
            arm.update(trials=deepcopy(report["baseline"]), summary=summary, difference_pp=0)
        arm = next(arm for arm in report["variants"] if arm["intervention"]["context_id"].endswith("dispatch-guide"))
        for trial, changed in zip(arm["trials"], outcomes, strict=True):
            if changed:
                trial["choice"]["outcome"] = "different answer"
        report["pair"]["interaction_pp"] = 0
        send(base, "/api/replay-reports", report, expected=201)
    def check(changed, held, mixed, unresolved):
        edits = send(base, "/api/sidecar/preview", {"request":pending})[1]["history"]["edits"]
        tested, = [e for e in edits if e["example"]["source"] == "dispatch-guide"]
        assert tested["requests"] == tested["equal"] == 2 and tested["lower"] == 0
        assert tested["proposals"] == dict(changed=changed, held=held, mixed=mixed, unresolved=unresolved), tested
        if changed:
            assert edits[0] == tested
            e = tested["example"]
            card, = get(base, "findings", group=e["group"], trace=e["trace"], call=e["call"], decision=e["decision"],
                        replay=e["report"], context=e["context"])["rows"]
            assert card["evidence"]["detail"]["response_pair"]["changed"] and card["evidence"]["detail"]["effect_pp"] == 0
    for r in saved:
        replay(r, "-change", [True, True])
    check(2, 0, 0, 0)
    revised = deepcopy(pending)
    attrs = revised["attributes"]
    body = json.loads(attrs["nexusweave.request.body"])
    source = attrs["nexusweave.context.semantics"]["request_sources"][0]
    source["id"] = "this-call-only"
    message = body["messages"][1]
    message["content"] = "Dispatch guide: Route Érable parcels to bay OISEAU."
    fragment = source["request_fragment"]["text_range"]
    fragment["end_byte"] = len(message["content"].encode())
    attrs["nexusweave.request.body"] = json.dumps(body, ensure_ascii=False)
    plan = send(base, "/api/sidecar/preview", {"request":revised})[1]["history"]["experiment"]
    edit, = plan["interventions"]
    encode = lambda doc: json.dumps(doc, ensure_ascii=False, separators=(",",":")).encode()
    assert plan["baseline_request_sha256"] == sha256(encode(body)).hexdigest()
    assert edit["context_id"] == "request-source:this-call-only" and edit["text_range"] == fragment
    message["content"] = "Dispatch guide: " + edit["replacement"]
    assert plan["edited_request_sha256"] == sha256(encode(body)).hexdigest()
    fragment["end_byte"] = len(message["content"].encode())
    attrs["nexusweave.request.body"] = json.dumps(body)
    skipped = send(base, "/api/sidecar/preview", {"request": revised})[1]["history"]["experiment"]
    assert skipped == {"status": "already_applied", "baseline_request_sha256": plan["edited_request_sha256"]}
    replay(saved[0], "-held", [False, False])
    check(1, 0, 1, 0)
    replay(saved[1], "-variable", [False, True])
    check(0, 0, 1, 1)


def check_current_history_test(binary):
    with server(binary, None) as base:
        folder = ROOT / "examples/redundant-routing"
        saved = json.loads((folder / "ash-history-check.json").read_text())
        original = json.loads((folder / "candidate-checks/ash-routing-note.json").read_text())["request"]
        def preview(request, shadow):
            return send(base, "/api/sidecar/preview", {"request":request, "shadow":shadow})[1]
        request = deepcopy(original)
        request["attributes"]["nexusweave.prefill.probe"] = saved["prefill"]
        result = preview(request, saved["shadow"])["history"]["experiment"]
        trial, = result["proposal_tests"]
        assert result["first_token_js_bits"] == saved["prefill"]["scores"]["effects"][0]["js_bits"]
        assert trial["pair"]["changed"] is False and trial["pair"]["original"]["outcome_preview"] == "UNKNOWN"
        changed = deepcopy(saved["shadow"])
        changed["effects"][0]["trials"][0]["choice"]["outcome"] = "another answer"
        assert preview(original, changed)["history"]["experiment"]["proposal_tests"][0]["pair"]["changed"] is True
        other = deepcopy(changed)
        arm = other["effects"][0]
        arm["intervention"]["replacement"] = "Different wording"
        body = json.loads(original["attributes"]["nexusweave.request.body"])
        body["messages"][2]["content"] = "Dispatch guide: Different wording"
        arm["request_sha256"] = sha256(json.dumps(body,separators=(",",":")).encode()).hexdigest()
        unrelated = preview(original, other)
        assert not unrelated["unverified_probes"] and not unrelated["history"]["experiment"]["proposal_tests"]
        other["captured_request_sha256"] = "0" * 64
        stale = preview(original, other)
        assert stale["unverified_probes"] and not stale["history"]["experiment"]["proposal_tests"]


def check_proposal_coverage(binary, db):
    folder = ROOT / "examples/redundant-routing"
    saved = json.loads((folder / "ash-history-check.json").read_text())["shadow"]
    request = json.loads((folder / "candidate-checks/ash-routing-note.json").read_text())["request"]
    org = "proposal-coverage"
    def observed(name, thread, report):
        span = case_span(name, "undeclared", org=org)
        fields = {**request["attributes"], "gen_ai.operation.name": "chat", "gen_ai.conversation.id": thread,
                  "nexusweave.organization.id": org, "nexusweave.shadow_probes": [report]}
        span["attributes"] = attrs({key: value if isinstance(value, str) else json.dumps(value) for key, value in fields.items()})
        return span
    different = deepcopy(saved)
    different["effects"][0]["trials"][0]["choice"]["outcome"] = "another answer"
    varying = deepcopy(different)
    varying["baseline"] *= 2
    varying["effects"][0]["trials"] += deepcopy(saved["baseline"])
    for summary in [varying["baseline_summary"], varying["effects"][0]["summary"]]:
        summary.update(attempted=2, completed=2)

    mixed = observed("mixed-changed", "mixed", different)
    held = observed("held", "held", saved)
    cases = [held, mixed, observed("mixed-held", "mixed", saved), observed("no-pair", "varied", varying)]
    with server(binary, db, "--capture-content") as base:
        send_traces(base, export(*cases))
        send_traces(base, export(held))
        impact = get(base, "impact", org=org)
        assert impact["proposal_outcomes"] == {"both": 1, "held_only": 1, "changed_only": 0, "no_pair": 1}, impact
        assert impact["changed"] == 1 and impact["evidence_counts"]["tested"] == 2
        changed, = get(base, "proposal-conversations", result="both", org=org)["rows"]
        assert changed["thread"] == "mixed" and set(changed["examples"]) == {"changed", "held"}
        for outcome, example in changed["examples"].items():
            card, = get(base, "findings", trace=example["trace"], call=example["call"], group=example["group"],
                        report=example["report"], factor=example["factor"])["rows"]
            assert example["witness"]["pair"] and card["evidence"]["detail"]["response_pair"]["changed"] == (outcome == "changed")
        changed_span = {"trace": "otlp:" + mixed["traceId"], "kind": "shadow"}
        assert get(base, "impact", **changed_span)["proposal_outcomes"]["changed_only"] == 1
        subset, = get(base, "proposal-conversations", result="changed_only", **changed_span)["rows"]
        assert subset["examples"] == {"changed": changed["examples"]["changed"]}
        row, = get(base, "proposal-conversations", result="no_pair", org=org)["rows"]
        assert row["thread"] == "varied" and row["examples"]["no_pair"]["witness"] is None
        assert not get(base, "proposal-conversations", result="held_only", org="unrelated")["rows"]
        send(base, "/api/sidecar/proposal-conversations?result=unknown", expected=400)
        factor, = get(base, "factors", kind="shadow", trace="otlp:" + held["traceId"])["rows"]
        assert factor["signal"] == "equal" and factor["changed"] == 0
        only, = get(base, "findings", kind="shadow", trace="otlp:" + held["traceId"])["rows"]
        assert only["action"]["title"] == "Test another source on this request"
        trace = get(base, "trace", id="otlp:" + held["traceId"])
        check, = trace["target_checks"][held["spanId"]]
        assert check["edits"][0]["status"] == "missed" and not check["edits"][0]["changed"]
    # Version 15 retained the raw shadow report but never emitted a finding for matching proposals.
    with sqlite3.connect(db) as conn:
        conn.execute("DELETE FROM observations WHERE kind='shadow'")
        conn.execute("PRAGMA user_version=15")
    with server(binary, db, "--capture-content") as base:
        assert get(base, "impact", org=org) == impact
        assert get(base, "proposal-conversations", result="both", org=org)["rows"] == [changed]


def check_target_token_scores(base):
    envelope = json.loads((ROOT / "examples/context-screening/replay/fresh-target-tokens.json").read_text())
    def preview(payload):
        return send(base, "/api/sidecar/preview", payload)[1]

    checked, = preview(envelope)["target_checks"]
    path, = checked["token_paths"]
    assert path["continuation"] == "45" and checked["joint"]["status"] == "missed"
    assert [c["first_lower"] for c in path["conditions"]] == [0, 0, 0, None]
    assert checked["next"] == "Inspect the text after the target answer"
    scores_only = preview({key: envelope[key] for key in ("request", "candidate")})
    alternatives, = scores_only["continuation_paths"]
    assert not scores_only["target_checks"] and [p["continuation"] for p in alternatives] == ["45", "30"]
    assert alternatives[0] == path

    for field in ("step_sum", "shared_max", "joint_hash"):
        altered = deepcopy(envelope)
        candidate = altered["candidate"]
        step = candidate["scores"]["conditions"][0]["steps"][0][0]
        if field == "step_sum":
            step["log_prob"] -= 1
        elif field == "shared_max":
            step["top_log_prob"] -= 0.1
        else:
            candidate["joint_request_sha256"] = "0" * 64
        rejected = preview(altered)
        assert rejected["unverified_probes"] and not rejected["continuation_paths"]
        assert not rejected["target_checks"][0]["token_paths"]

    different = deepcopy(envelope)
    different["shadow"]["expected"]["outcome"] = "90"
    checked, = preview(different)["target_checks"]
    assert not checked["token_paths"] and checked["joint"]["status"] == "missed"


def check_predecision_recommendations(base):
    check_target_token_scores(base)
    fixture = ROOT / "examples/runtime-comparison/pending-routing"
    read = lambda name: json.loads((fixture / name).read_text())
    request, candidate, shadow = read("request.json"), read("report.json"), read("shadow-report.json")
    preview_path = "/api/sidecar/preview"
    before = get(base, "health")["coverage"]
    recommendation = send(base, preview_path, {"request": request, "candidate": candidate, "shadow": shadow})[1]
    result = recommendation["recommendations"]
    assert not recommendation["unverified_probes"]
    target, = recommendation["target_checks"]
    assert [a["control"] for a in target["edits"]] == [False, True]
    assert target["edits"][0]["edit"]["pointer"] == "/messages/1/content"
    assert target["match_rule"] == {"kind":"outcome_fields","pointers":["/product_id"]}
    assert [arm["status"] for arm in [target["baseline"], *target["edits"], target["joint"]]] == ["missed", "met", "missed", "met"]
    # Exercise the target policy through the same report verifier as a real producer.
    control_hit = deepcopy(shadow)
    control_hit["control_comparison"] = {"control_context_id": shadow["effects"][0]["intervention"]["context_id"],
                                         "target_minus_control_pp": -100.0}
    checked, = send(base, preview_path, {"request":request, "shadow":control_hit})[1]["target_checks"]
    assert checked["next"] == "Check why the negative control changed the target result"
    incomplete = deepcopy(shadow)
    incomplete["pair"].update(trials=[{"status":"failed"}], summary={"attempted":1, "completed":0, "matched":0},
                              difference_pp=None, interaction_pp=None)
    checked, = send(base, preview_path, {"request":request, "shadow":incomplete})[1]["target_checks"]
    assert checked["joint"]["status"] == "incomplete" and checked["joint"]["changed"] is None
    assert checked["next"].startswith("Complete the missing")
    for scenario, expected in [("window-earlier", "met"), ("window-followup", "missed"), ("window-unopened", "missed")]:
        checked, = get(base, "preview-example", scenario=scenario)["target_checks"]
        assert checked["expected"]["outcome_preview"] == "45"
        assert checked["joint"]["changed"] and checked["joint"]["status"] == expected
    stale_shadow = deepcopy(shadow)
    stale_shadow["captured_request_sha256"] = "0" * 64
    rejected_shadow = send(base, preview_path, {"request":request, "shadow":stale_shadow})[1]
    assert rejected_shadow["unverified_probes"] and not rejected_shadow["target_checks"]
    scored_dir = ROOT / "examples/redundant-routing/candidate-checks"
    for example, expected in [("maple-dispatch-guide", "routing-note"), ("ash-routing-note", "routing-note")]:
        envelope = json.loads((scored_dir / (example + ".json")).read_text())
        cards = send(base, preview_path, envelope)[1]["recommendations"]
        first = cards[0]["detail"]
        assert first["sources"][0]["source"] == expected and first["baseline"] * first["edited"] < 0
    warranty = get(base, "preview-example", scenario="warranty")["recommendations"][0]
    assert warranty["action"]["title"] == "Check which other answers are plausible"
    assert abs(warranty["detail"]["edited_outside_pair"] - 0.5972319211461123) < 1e-8
    route = next(item for item in result if item["kind"] == "candidate" and item["detail"]["sources"][0]["source"] == "request-type")
    assert route["detail"]["baseline"] > 0 > route["detail"]["edited"]
    assert route["detail"]["other_edits"][0]["source"] == "store-name"
    assert abs(route["detail"]["other_edits"][0]["change"]) < 0.1
    assert any(item["kind"] == "shadow" and item["detail"]["effect_pp"] == 100 for item in result)
    assert get(base, "health")["coverage"] == before

    tampered = deepcopy(candidate)
    tampered["captured_request_sha256"] = "0" * 64
    rejected = send(base, preview_path, {"request": request, "candidate": tampered})[1]
    assert rejected["unverified_probes"] and not any(item["kind"] == "candidate" for item in rejected["recommendations"])

    observed = case_span("measured-routing", "undeclared", org="candidate-validation")
    body = json.loads(request["attributes"]["nexusweave.request.body"])
    messages = [{"role": item["role"], "parts": [{"type": "text", "content": item["content"]}]}
                for item in body["messages"]]
    observed["attributes"] = attrs({
        "gen_ai.operation.name": "chat",
        "nexusweave.organization.id": "candidate-validation",
        "nexusweave.candidate_probes": json.dumps([candidate]),
        "gen_ai.input.messages": json.dumps(messages),
        **{key: value if isinstance(value, str) else json.dumps(value) for key, value in request["attributes"].items()},
    })
    send_traces(base, export(observed))
    published = get(base, "findings", kind="candidate", org="candidate-validation")["rows"]
    impact = get(base, "impact", org="candidate-validation")
    assert impact["conversations"] == 0 and impact["unlinked_requests"] == 1, impact
    sibling = deepcopy(observed)
    sibling["spanId"] = "3333333333333333"
    send_traces(base, export(sibling))
    assert get(base, "impact", org="candidate-validation")["unlinked_requests"] == 2
    assert published[0]["action"]["basis"] == "Measured preference at a fixed prefix"
    assert published[0]["evidence"]["detail"]["baseline"] > 0 > published[0]["evidence"]["detail"]["edited"]
    assert not get(base, "findings", kind="duplicate", org="candidate-validation")["rows"]


def check_activation(base, producer_python):
    output = subprocess.check_output([producer_python, str(ROOT / "examples/send_otlp_activation.py"), "--url", base], text=True)
    linked = parse_qs(urlsplit(output.strip()).fragment)
    trace = linked["run"][0]
    assert get(base, "trace", id=trace)["activation_reports"] == 1
    token = get(base, "findings", kind="token")["rows"][0]
    assert token["evidence"]["trace_id"] == trace and len(token["evidence"]["detail"]["landmark"]["positions"]) > 1
    finding = token["evidence"]["detail"]
    location = finding["location"]
    query = dict(trace=trace, call=location["call_id"], decision=location["decision_id"], report=finding["report_id"],
                 group=token["id"], factor=finding["factor"], block=0, org="qwen-demo-org")
    page = get(base, "token-path", **query)
    assert page["total_output_tokens"] == len(page["tokens"]) == 16
    assert len(page["edits"]) == 3 and page["captured_sources"] >= 5
    assert {"format-instruction", "question"} <= {c["source"] for c in page["untested_examples"]}
    edits = {e["source"]["source"]: e for e in page["edits"]}
    assert edits["returns-policy"]["peak"]["token"] == 5 and edits["refund-policy"]["peak"]["token"] == 13
    assert not edits["store-name"]["output_changed"] and edits["returns-policy"]["output_changed"]
    scope = dict(org="qwen-demo-org", level=1, by="edit", kind="token")
    groups = get(base, "outcome-factors", **scope)["rows"]
    assert {r["sources"][0]["source"] for r in groups} == {"returns-policy", "refund-policy"}
    for row in groups:
        member, = get(base, "conversation-outcomes", **scope, cohort_factor=row["factor"])["rows"]
        original = edits[row["sources"][0]["source"]]
        assert member["witness"]["generation"] == original["context_id"] and original["generated_choice_changed"]
        assert member["report"] == finding["report_id"] and member["call"] == location["call_id"]
    assert get(base, "token-path", **query, offset=15)["tokens"] == page["tokens"][-1:]
    for mismatch in ({"org":"absent"}, {"decision":"absent"}, {"factor":"absent"}, {"group":"absent"}, {"before":"2000-01-01"}):
        send(base, "/api/sidecar/token-path?" + urlencode({**query, **mismatch}), expected=400)
    details = send(base, "/api/inspection?" + urlencode({"run": trace}))[1]
    assert details["activation_reports"][0]["location"]["run_id"] == trace


def check_external_batch(binary, folder):
    fixtures = ROOT / "examples/context-screening/replay"
    evidence, case = fixtures / "evidence.json", fixtures / "case.json"
    plan_path = folder / "plan.json"
    subprocess.run([str(binary), "plan-replay", str(evidence), str(case), str(plan_path)], check=True)
    plan = json.loads(plan_path.read_text())
    saved = json.loads((fixtures / "report.json").read_text())
    trials_by_condition = [saved["baseline"], *(arm["trials"] for arm in saved["variants"]), saved["pair"]["trials"]]
    outcomes = [
        {"context_ids": condition["context_ids"], "request_sha256": condition["request_sha256"], "trials": trials}
        for condition, trials in zip(plan["conditions"], trials_by_condition, strict=True)
    ]
    batch = {"version": 1, "plan_id": plan["plan_id"], "conditions": list(reversed(outcomes))}
    trials_path, report_path = folder / "trials.json", folder / "report.json"
    trials_path.write_text(json.dumps(batch))
    command = [str(binary), "score-replay", str(evidence), str(case), str(trials_path)]
    subprocess.run([*command, str(report_path)], check=True)
    assert json.loads(report_path.read_text()) == saved
    batch["conditions"][0]["request_sha256"] = "1" * 64
    trials_path.write_text(json.dumps(batch))
    failure = subprocess.run([*command, str(folder / "rejected.json")], capture_output=True)
    assert failure.returncode != 0 and b"request digest" in failure.stderr


# The labels answer whether an engineer should investigate the issue. Separately
# recorded precedence labels the benign structured disagreement as low priority.
COHORTS = {
    "development": ("refund_cap", "support_region", "missing_exception", "intended_escalation", "copied_notice"),
    "heldout": ("recall_status", "retention_window", "hidden_override", "accepted_precedence", "repeat_reminder"),
}


def rank(base, cohort, send_trace=send_traces):
    primary, secondary, missing, benign, duplicate = COHORTS[cohort]
    labels = {primary, secondary, missing}
    for name, mode, count, shift in [
        (primary, "conflict", 4, 0), (secondary, "conflict", 3, 200),
        (missing, "undeclared", 2, 300), (benign, "conflict", 1, 6 * 86400),
        (duplicate, "duplicate", 2, 5 * 86400),
    ]:
        spans = (case_span(name, mode, number, org=cohort, shift=shift) for number in range(count))
        send_trace(base, export(*spans))
    rows = get(base, "findings", org=cohort)["rows"]
    impact = get(base, "impact", org=cohort)
    assert impact["conversations"] == 4 and impact["changed"] == 0, impact
    assert impact["replay_outcomes"] == {"mixed": 0, "changed_only": 0, "equal_only": 0, "unmeasured": 4}
    factors = get(base, "factors", org=cohort)["rows"]
    assert impact["factors"] == len(factors)
    assert get(base, "impact", org=cohort, user="absent")["conversations"] == 0
    assert get(base, "factors", org=cohort, user="absent")["rows"] == []
    sample = get(base, "occurrences", org=cohort, group=rows[0]["id"])["rows"]
    assert len(sample) == rows[0]["occurrences"]
    assert len({row["thread_key"] for row in sample}) == rows[0]["threads"]
    assert len(rows) == 4, rows
    assert rows[0]["evidence"]["detail"]["rule"] == primary
    assert len(rows[0]["trend"]) == 4
    assert get(base, "findings", org=cohort, group=rows[0]["id"], after="2026-10-13", before="2026-10-15")["rows"][0]["traces"] == 2
    assert get(base, "findings", org=cohort, group=rows[0]["id"], user="unseen")["rows"] == []
    assert len(get(base, "threads", org=cohort)["rows"]) == 4
    if cohort == "heldout":
        rows.extend(row for row in get(base, "findings", kind="replay")["rows"] if row["kind"] == "replay")
        labels.update({"measured source edit", "measured joint edit"})
        rows.sort(key=lambda row: (row["priority"],
                                  -row["threads"], -row["traces"], -(row["latest"] or 0), row["id"]))
    def name(row):
        detail = row["evidence"]["detail"]
        if row["kind"] == "replay":
            if detail["edit"] == "joint":
                return "measured joint edit"
            return "measured source edit" if detail["effect_pp"] != 0 else "measured equality"
        return detail.get("rule", duplicate)

    scores = {}
    observed = {name(row) for row in rows}
    for method, ordering in [
        ("ranked", rows),
        ("rules_only", sorted((row for row in rows if row["kind"] == "conflict"), key=lambda row: -row["latest"])),
        ("newest_first", sorted(rows, key=lambda row: -(row["latest"] or 0))),
    ]:
        top = [name(row) for row in ordering[:5]]
        scores[method] = {
            "true_at_5": len(set(top) & labels), "shown": len(top), "labeled_issues": len(labels),
            "precision_among_shown": round(len(set(top) & labels) / len(top), 3),
            "coverage": round(len({name(row) for row in ordering} & labels) / len(labels), 3),
            "highest_false_positive": next((label for label in top if label not in labels), None),
        }
    return {"findings": scores, "missed": sorted(labels - observed)}


def load(base, clients, batches, batch_size, payload_bytes):
    org = "sidecar-overload"
    inputs = []
    for index in range(batches):
        spans = []
        for number in range(batch_size):
            span = case_span(f"load-{index}", "conflict", number, org=org)
            if payload_bytes:
                span["attributes"].extend(attrs({"gen_ai.system_instructions": json.dumps([
                    {"type": "text", "content": "x" * payload_bytes}
                ])}))
            spans.append(span)
        inputs.append(export(*spans))

    def submit(payload):
        started = time.monotonic()
        code, _ = send(base, "/v1/traces", payload, expected=None)
        return code, (time.monotonic() - started) * 1000

    initial = get(base, "health")
    started = time.monotonic()
    with ThreadPoolExecutor(max_workers=clients) as pool:
        outcomes = list(pool.map(submit, inputs))
    elapsed = time.monotonic() - started
    final = get(base, "health")
    codes = sorted({code for code, _ in outcomes})
    timings = sorted(ms for code, ms in outcomes if code == 200)
    assert set(codes) <= {200, 503} and timings, codes
    drops = final["queue_drops"] - initial["queue_drops"]
    assert drops == sum(code == 503 for code, _ in outcomes)
    assert final["queue_depth"] == 0
    recovered = 0
    recovery_started = time.monotonic()
    for payload, (code, _) in zip(inputs, outcomes, strict=True):
        if code == 503:
            send(base, "/v1/traces", payload)
            recovered += 1
    recovery_seconds = time.monotonic() - recovery_started
    coverage = get(base, "health", org=org)["coverage"]
    assert coverage["traces"] == batches * batch_size
    reads = []
    cohort_reads = []
    for _ in range(10):
        start = time.monotonic()
        impact = get(base, "impact", org=org)
        factors = get(base, "factors", org=org)
        reads.append(1000 * (time.monotonic() - start))
        assert impact == {"conversations": batch_size, "changed": 0, "unlinked_requests": 0, "factors": batches,
                          "captured_conversations": batch_size, "without_findings": 0, "without_findings_with_screen_suggestion": 0,
                          "evidence_counts": {"proposal": 0, "rate": 0, "score": 0, "tested": 0, "inputs": batch_size},
                          "proposal_outcomes": {"both": 0, "held_only": 0, "changed_only": 0, "no_pair": batch_size},
                          "replay_outcomes": {"mixed": 0, "changed_only": 0, "equal_only": 0, "unmeasured": batch_size}}
        assert len(factors["rows"]) == min(batches, 15)
        start = time.monotonic()
        groups = get(base, "outcome-factors", org=org, level=5)
        first = groups["rows"][0]
        members = get(base, "conversation-outcomes", org=org, level=5, cohort_factor=first["factor"])
        cohort_reads.append(1000 * (time.monotonic() - start))
        assert first["conversations"] == first["cohort_size"] == batch_size
        assert len(members["rows"]) == min(batch_size, 15)
    return {"responses": {code: sum(status == code for status, _ in outcomes) for code in codes},
            "wall_s": round(elapsed, 3), "p95_successful_export_ms": round(timings[int((len(timings) - 1) * 0.95)], 2),
            "last_processing_lag_ms": final["processing_lag_ms"], "batch_size": batch_size,
            "clients": clients, "extra_bytes_per_span": payload_bytes, "retry_recovered_batches": recovered,
            "retry_wall_s": round(recovery_seconds, 3), "retained_traces": coverage["traces"],
            "overview_read_pairs": len(reads), "median_sequential_overview_ms": round(median(reads), 2),
            "median_source_group_and_members_ms": round(median(cohort_reads), 2)}


def measure_token_pages(binary):
    """Time warm loopback reads, including client-side JSON decoding."""
    with server(binary, None) as base:
        pages = []
        edit_groups = get(base, "outcome-factors", level=1, by="edit", kind="token")["rows"]
        assert {r["sources"][0]["source"] for r in edit_groups} == {"returns-policy","refund-policy","request-type","product-id"}
        for card in get(base, "findings", kind="token")["rows"]:
            detail = card["evidence"]["detail"]
            location = detail["location"]
            for offset in range(0, detail["landmark"]["total_tokens"], 16):
                pages.append(dict(trace=location["run_id"], call=location["call_id"],
                                  decision=location["decision_id"], report=detail["report_id"],
                                  group=card["id"], block=detail["landmark"]["block"], offset=offset))
        assert len(pages) == 3, "Expected one Qwen page and two MLX pages"
        for page in pages:
            assert get(base, "token-path", **page)["tokens"]
        timings = []
        for _ in range(25):
            for page in pages:
                start = time.perf_counter()
                get(base, "token-path", **page)
                timings.append((time.perf_counter() - start) * 1000)
    return {"warm_reads": len(timings), "distinct_pages": len(pages),
            "median_ms": round(median(timings), 3), "p95_ms": round(sorted(timings)[ceil(0.95 * len(timings)) - 1], 3)}


def measure_dense_screens(binary):
    # Force every assessed pair to receive a flag so the index reaches its supported per-call limit.
    scores = {"contradiction":1.0, "entailment":0.0, "neutral":0.0}
    pairs = [{"sources":[a,b], "status":"scored", "suggestion":"possible_conflict",
              "left_to_right":scores, "right_to_left":scores} for a,b in list(combinations(range(24),2))[:256]]
    timings = []
    with tempfile.TemporaryDirectory() as directory, server(binary, Path(directory) / "dense.sqlite", "--capture-content") as base:
        for call in range(32):
            messages, sources = [], []
            for i in range(24):
                text = f"Synthetic instruction {call}/{i} for limit testing."
                messages.append({"role":"developer", "parts":[{"type":"text","content":text}]})
                sources.append({"context_id":f"input:{i}:0", "kind":"text", "role":"developer",
                                "text_sha256":sha256(text.encode()).hexdigest()})
            report = {"version":1, "selection":{"version":1,"pair_limit":256}, "sources":sources,
                      "scorer":{"model_id":"synthetic-load","revision":"v1","runtime":"fixture"},
                      "total_pairs":276, "pairs":pairs}
            span = case_span("dense","undeclared",number=call)
            span["attributes"] = attrs({"gen_ai.operation.name":"chat","gen_ai.conversation.id":f"c{call}",
                "gen_ai.input.messages":json.dumps(messages),"nexusweave.context_screening":json.dumps(report)})
            send_traces(base, export(span))
        assert len(get(base, "screened-pairs", offset=8191)["rows"]) == 1
        for iteration in range(21):
            start = time.perf_counter()
            page = get(base, "screened-pairs")
            elapsed = (time.perf_counter() - start) * 1000
            assert len(page["rows"]) == page["next_offset"] == 15
            if iteration:
                timings.append(elapsed)
    return {"distinct_flagged_pairs":8192, "warm_first_pages":len(timings),
            "median_ms":round(median(timings),3), "p95_ms":round(sorted(timings)[ceil(len(timings)*.95)-1],3)}


def measure_screened_pairs(binary):
    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare
    timings = []
    with tempfile.TemporaryDirectory() as directory, server(binary, Path(directory) / "pairs.sqlite", "--capture-content") as base:
        for i in range(128):
            trace, report = prepare(f"journal-{i}", followup=i % 2 == 0)
            send_traces(base, trace)
            if i % 4 < 2:
                send(base, "/api/replay-reports", report, expected=201)
        for iteration in range(21):
            start = time.perf_counter()
            group, = get(base, "screened-pairs")["rows"]
            query = {"pattern":group["pattern"], "relation":group["relation"]}
            page = get(base, "screened-pair-examples", **query)
            elapsed = (time.perf_counter() - start) * 1000
            assert group["conversations"] == 128 and group["tested"] == 64
            assert all(x == {"changed":32,"held":32} for x in group["source_outcomes"].values())
            assert len(page["rows"]) == 15 and all(not row["comparisons"] for row in page["rows"])
            if iteration:
                timings.append(elapsed)
        threads = []
        while True:
            threads.extend(row["thread"] for row in page["rows"])
            if page["next_offset"] is None:
                break
            page = get(base, "screened-pair-examples", offset=page["next_offset"], **query)
        assert len(threads) == len(set(threads)) == 128
    return {"flagged_conversations":128, "tested":64, "warm_read_pairs":len(timings),
            "median_ms":round(median(timings),3), "p95_ms":round(sorted(timings)[ceil(len(timings)*.95)-1],3)}


def measure_population(binary):
    timings = []
    with tempfile.TemporaryDirectory() as directory, server(binary, Path(directory) / "population.sqlite", "--capture-content") as base:
        for start in range(0, 256, 16):
            spans = [case_span("population-cost", "conflict" if i % 2 else "undeclared", i) for i in range(start, start + 16)]
            send_traces(base, export(*spans))
        for iteration in range(21):
            start = time.perf_counter()
            impact = get(base, "impact")
            page = get(base, "conversations-without-findings")
            elapsed = (time.perf_counter() - start) * 1000
            assert impact["captured_conversations"] == 256 and impact["without_findings"] == impact["conversations"] == 128
            assert len(page["rows"]) == page["next_offset"] == 15
            if iteration:
                timings.append(elapsed)
        threads = [r["thread"] for r in page["rows"]]
        while page["next_offset"] is not None:
            page = get(base, "conversations-without-findings", offset=page["next_offset"])
            threads.extend(r["thread"] for r in page["rows"])
        assert len(threads) == len(set(threads)) == 128
    return {"captured":256, "without_findings":128, "warm_read_pairs":len(timings),
            "median_ms":round(median(timings), 3), "p95_ms":round(sorted(timings)[ceil(len(timings) * .95) - 1], 3)}


def measure_history_preview(binary):
    folder = ROOT / "examples/redundant-routing"
    request = json.loads((folder / "candidate-checks/ash-routing-note.json").read_text())["request"]
    saved = json.loads((folder / "ash-history-check.json").read_text())
    measured = deepcopy(request)
    measured["attributes"]["nexusweave.prefill.probe"] = saved["prefill"]
    cases = {"history": {"request": request}, "with_current_trial": {"request": measured, "shadow": saved["shadow"]}}
    timings = {name: [] for name in cases}
    with server(binary, None) as base:
        for round_index in range(26):
            for name, envelope in cases.items():
                start = time.perf_counter()
                result = send(base, "/api/sidecar/preview", envelope)[1]["history"]["experiment"]
                elapsed = (time.perf_counter() - start) * 1000
                assert result["status"] == "ready" and result["total_proposal_tests"] == int(name == "with_current_trial")
                if round_index:
                    timings[name].append(elapsed)
    return {name: {"warm_requests": len(ms), "median_ms": round(median(ms), 3),
                   "p95_ms": round(sorted(ms)[ceil(len(ms) * 0.95) - 1], 3)} for name, ms in timings.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/nexusweave")
    parser.add_argument("--sdk-python", help="Interpreter with OpenTelemetry SDK and OTLP/HTTP exporter installed")
    parser.add_argument("--overload-url", help="Run load only against an isolated persistent sidecar")
    parser.add_argument("--token-page-cost", action="store_true", help="Measure warm reads of the two instrumented demo answers")
    parser.add_argument("--dense-screen-cost", action="store_true", help="Time a page of groups at 256 synthetic flags per call")
    parser.add_argument("--screen-cost", action="store_true", help="Time screened-pair summaries and example pages on 128 conversations")
    parser.add_argument("--population-cost", action="store_true", help="Time the overview and first page of conversations without a finding")
    parser.add_argument("--history-cost", action="store_true", help="Time the pending Ash request with and without its saved trial")
    parser.add_argument("--clients", type=int, default=24)
    parser.add_argument("--batches", type=int, default=192)
    parser.add_argument("--batch-size", type=int, default=12)
    parser.add_argument("--payload-bytes", type=int, default=4096)
    args = parser.parse_args()
    if args.dense_screen_cost:
        print(json.dumps(measure_dense_screens(args.binary)))
        return
    if args.screen_cost:
        print(json.dumps(measure_screened_pairs(args.binary)))
        return
    if args.population_cost:
        print(json.dumps(measure_population(args.binary)))
        return
    if args.history_cost:
        print(json.dumps(measure_history_preview(args.binary), indent=2))
        return
    if args.token_page_cost:
        print(json.dumps(measure_token_pages(args.binary), indent=2))
        return
    if args.overload_url:
        print(json.dumps(load(args.overload_url, args.clients, args.batches, args.batch_size, args.payload_bytes), indent=2))
        return
    with tempfile.TemporaryDirectory() as directory:
        folder = Path(directory)
        check_capture(args.binary, folder, args.sdk_python)
        if args.sdk_python:
            check_first_recommendation(args.binary, folder, args.sdk_python)
        check_screen_suggestions(args.binary, folder)
        check_conversation_population(args.binary, folder / "population.sqlite")
        check_thread_calls(args.binary, folder)
        check_tool_handoffs(args.binary, folder / "tool-handoffs.sqlite")
        check_declared_conflict_replays(args.binary, folder)
        check_external_batch(args.binary, folder)
        check_current_history_test(args.binary)
        check_proposal_coverage(args.binary, folder / "proposal-coverage.sqlite")
        with server(args.binary, folder / "prior-proposals", "--capture-content") as base:
            check_history_proposals(base)
        captured = folder / "scoped.sqlite"
        with server(args.binary, captured, "--capture-content") as base:
            fidelity = check_replay(base)
            development = rank(base, "development")
            heldout = rank(base, "heldout")
            check_predecision_recommendations(base)
            chat = case_span("conversation-history", "duplicate", org="dialogue")
            for field in chat["attributes"]:
                if field["key"] == "gen_ai.input.messages":
                    entries = json.loads(field["value"]["stringValue"])
                    for item in entries:
                        item["role"] = "assistant"
                    field["value"]["stringValue"] = json.dumps(entries)
            chat["attributes"] = [field for field in chat["attributes"] if field["key"] != "nexusweave.context.semantics"]
            send_traces(base, export(chat))
            repeated = get(base, "findings", org="dialogue")["rows"][0]
            assert repeated["evidence"]["detail"]["dialogue_only"] and repeated["priority"] == 5
            assert get(base, "factors", org="dialogue")["rows"][0]["signal"] == "dialogue"
            if args.sdk_python:
                check_activation(base, args.sdk_python)
                changed = get(base, "conversation-outcomes", org="qwen-demo-org", level=1)
                assert changed["rows"]
        if args.sdk_python:
            with sqlite3.connect(captured) as sql:
                sql.execute("PRAGMA user_version = 12")
                sql.execute("UPDATE observations SET detail=json_remove(detail,'$.landmark.generated_choice_changed') WHERE kind='token'")
            with server(args.binary, captured, "--capture-content") as base:
                assert get(base, "conversation-outcomes", org="qwen-demo-org", level=1) == changed
        with sqlite3.connect(captured) as sql:
            sql.execute("PRAGMA user_version = 0")
            sql.execute("DELETE FROM observations WHERE kind IN ('candidate','token')")
        with server(args.binary, captured, "--capture-content") as base:
            assert get(base, "findings", kind="replay")["rows"]
            assert get(base, "findings", kind="candidate")["rows"]
            if args.sdk_python:
                assert get(base, "findings", kind="token")["rows"]
        with sqlite3.connect(captured) as sql:
            sql.execute("PRAGMA user_version = 5")
            sql.execute("UPDATE observations SET detail=json_remove(detail,'$.baseline_outside_pair','$.edited_outside_pair') WHERE kind='candidate'")
        with server(args.binary, captured, "--capture-content") as base:
            upgraded = get(base, "findings", kind="candidate")["rows"]
            assert upgraded
            for card in upgraded:
                detail = card["evidence"]["detail"]
                assert 0 <= detail["baseline_outside_pair"] <= 1 and 0 <= detail["edited_outside_pair"] <= 1
        representative = folder / "representative.sqlite"
        with server(args.binary, representative, "--capture-content") as base:
            check_replay_representative(base)
            check_cohort_paging(base)
            check_replacement_comparison(base)
            check_replay_variation(base)
            joint_group, joint_rows = check_joint_proposals(base)
            check_order_tests(base)
            pairs = get(base, "order-tests", org="order-check")
            proposal_rows = get(base, "conversation-outcomes", level=1)
            assert proposal_rows["rows"]
        with sqlite3.connect(representative) as sql:
            sql.execute("PRAGMA user_version = 13")
            sql.execute("UPDATE observations SET detail=json_remove(detail,'$.joint_pattern','$.joint_input') WHERE kind='replay'")
        with server(args.binary, representative, "--capture-content") as base:
            assert get(base, "findings", group=joint_group, org="joint-check")["rows"][0]["joint_patterns"] == joint_rows
        with sqlite3.connect(representative) as sql:
            sql.execute("PRAGMA user_version = 11")
            sql.execute("UPDATE observations SET detail=json_remove(detail,'$.response_pair') WHERE kind='replay'")
        with server(args.binary, representative, "--capture-content") as base:
            assert get(base, "conversation-outcomes", level=1) == proposal_rows
        with sqlite3.connect(representative) as sql:
            sql.execute("PRAGMA user_version = 3")
            sql.execute("UPDATE observations SET detail=json_remove(detail,'$.order_test') WHERE kind='replay'")
        with server(args.binary, representative, "--capture-content") as base:
            assert get(base, "order-tests", org="order-check") == pairs
        with sqlite3.connect(representative) as sql:
            sql.execute("PRAGMA user_version = 4")
            sql.execute("UPDATE observations SET detail=json_remove(detail,'$.prior') WHERE kind='replay'")
        lookup = json.loads((ROOT / "examples/redundant-routing/pending-maple.json").read_text())
        lookup["attributes"].update({"nexusweave.organization.id": "order-check", "nexusweave.user.id": "dispatch-guide",
                                     "nexusweave.history.source": "otlp"})
        with server(args.binary, representative, "--capture-content") as base:
            rebuilt = send(base, "/api/sidecar/preview", {"request": lookup})[1]["history"]
            assert rebuilt["edits"] and not rebuilt["truncated"], rebuilt
        with sqlite3.connect(representative) as sql:
            # Fill the most recent window with repeats of a single measured input.
            row = sql.execute("""SELECT o.trace_id,o.call_id,o.kind,o.group_id,o.detail FROM observations o
                JOIN traces t ON t.id=o.trace_id WHERE t.org='order-check' AND t.user='dispatch-guide'
                AND json_extract(o.detail,'$.prior.partition') IS NOT NULL LIMIT 1""").fetchone()
            sql.executemany("INSERT INTO observations VALUES (?,?,?,?,?,?,?)",
                            [(row[0], n, *row[1:4], 2147483647, row[4]) for n in range(10000,10513)])
        with server(args.binary, representative, "--capture-content") as base:
            bounded = send(base, "/api/sidecar/preview", {"request": lookup})[1]["history"]
            assert bounded["truncated"] and not bounded["edits"], bounded
        if args.sdk_python:
            # Repeat the live card, recurrence, and replay journeys using the
            # SDK's actual encoder, processor, and exporter.
            with server(args.binary, folder / "sdk.sqlite", "--capture-content") as base:
                sdk = sdk_sender(args.sdk_python)
                assert check_replay(base, sdk) == fidelity
                assert rank(base, "heldout", sdk) == heldout
        print(json.dumps({"capture_checks": "passed", "producer_sdk": bool(args.sdk_python), "recomputed_replay_deltas": fidelity,
                          "development": development, "heldout": heldout}, indent=2))


if __name__ == "__main__":
    main()
