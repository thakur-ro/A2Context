"""Verify the sidecar API with synthetic traces."""

import argparse
from contextlib import contextmanager
from copy import deepcopy
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
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
            "a2context.context.semantics": json.dumps({"version": 1, "context": declarations}),
            "a2context.organization.id": org,
            "a2context.user.id": "case-owner",
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
            sdk_sender(producer_python)(base, export(case_span("sdk-redaction", "conflict", 1, org="sdk-test")))
            assert get(base, "health")["coverage"]["inference_spans"] == 2
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
        if producer_python:
            assert get(base, "impact", org="sdk-test")["captured_conversations"] == 1


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
        span["attributes"] += attrs({"a2context.turn.id": f"turn-{index // 2}"})
        spans.append(span)
    tool = spans[17]
    tool["attributes"] = attrs({
        "gen_ai.operation.name": "execute_tool", "gen_ai.tool.name": "update_order",
        "gen_ai.tool.call.id": "order-write", "gen_ai.tool.call.result": "Order write rejected",
        "gen_ai.conversation.id": "thread-0", "a2context.organization.id": "thread-test",
        "a2context.user.id": "case-owner", "a2context.turn.id": "turn-8",
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
        [hit] = get(base, "activity-search", q="order writes rejected", org="thread-test")["rows"]
        assert (hit["trace"], hit["call"], hit["thread_id"]) == (rows[17]["trace"], rows[17]["call"], key)
        assert hit["dialogue"] == rows[17]["dialogue"]
        assert get(base, "activity-search", q="order rejected", org="another-org")["rows"] == []
        assert get(base, "activity-search", q="order rejected", after="2026-10-13")["rows"] == []
        assert get(base, "activity-search", q="order rejected cloudberry")["partial_match"]
        assert get(base, "activity-search", q="order rejected cloudberry")["minimum_matching_terms"] == 2
        assert get(base, "activity-search", q="rejected cloudberry")["minimum_matching_terms"] == 1
        assert get(base, "activity-search", q="order rejected cloudberry", match="all")["rows"] == []
        assert get(base, "recent-tool-names", org="thread-test")["tools"] == ["update_order"]
        assert get(base, "activity-search", q="update_order", tool="update_order")["rows"] == [hit]
        assert get(base, "activity-search", q="update_order", tool="unknown")["rows"] == []
        for restriction in [{"org": "another-org"}, {"after": "2026-10-13"}, {"user": "other"}]:
            assert get(base, "recent-tool-names", **restriction)["tools"] == []
        assert get(base, "activity-search", q='"*"')["rows"] == []
        unthreaded = deepcopy(tool)
        unthreaded.update(traceId=f"{391:032x}", spanId=f"{391:016x}")
        unthreaded["attributes"] = [row for row in unthreaded["attributes"] if row["key"] not in {
            "gen_ai.conversation.id", "gen_ai.tool.call.result",
        }] + attrs({"gen_ai.tool.call.result": "Unthreaded shipping receipt"})
        send_traces(base, export(unthreaded))
        [hit] = get(base, "activity-search", q="shipping receipt")["rows"]
        assert hit["call"] == unthreaded["spanId"] and hit["thread_id"] is None
        distractor = deepcopy(unthreaded)
        distractor.update(traceId=f"{392:032x}", spanId=f"{392:016x}")
        for field in distractor["attributes"]:
            if field["key"] == "gen_ai.tool.call.result":
                field["value"]["stringValue"] = "Moonbase departure only"
        send_traces(base, export(distractor))
        partial = get(base, "activity-search", q="shipping receipt moonbase")
        assert partial["partial_match"] and [row["call"] for row in partial["rows"]] == [unthreaded["spanId"]]
        focus = {"focus_trace": rows[-1]["trace"], "focus_call": rows[-1]["call"]}
        assert get(base, "thread-calls", thread=key, **focus) == second
        compact = get(base, "thread-calls", thread=key, limit=3, **focus)
        assert compact["offset"] == 18 and compact["rows"] == rows[18:]
        # The selected call can fall on either side of an ordinary page boundary.
        for index in (0, 14, 15, len(rows) - 1):
            current = rows[index]
            focus = {"focus_trace": current["trace"], "focus_call": current["call"]}
            for direction, start, stop in [("before", max(0, index - 1), index + 1), ("after", index, index + 2)]:
                pair = get(base, "thread-calls", thread=key, limit=2, focus_window=direction, **focus)
                assert pair["rows"] == rows[start:stop] and pair["offset"] == start
                local = [r for r in rows if r["trace"] == current["trace"]]
                pos = local.index(current)
                trace_page = get(base, "thread-calls", trace=current["trace"], limit=2, focus_window=direction, **focus)
                assert trace_page["rows"] == (local[max(0,pos-1):pos+1] if direction == "before" else local[pos:pos+2])
            assert get(base, "thread-calls", thread=key, workflow="other", focus_window="before", **focus)["rows"] == []
        for query in [{"focus_window":"before"}, {**focus,"focus_window":"elsewhere"}]:
            assert send(base, "/api/sidecar/thread-calls?" + urlencode({"thread":key,**query}), expected=None)[0] == 400
        for restriction in [{"user": "other"}, {"org": "other"}, {"workflow": "other"},
                            {"meta_key": "deployment.environment.name", "meta_value": "other"},
                            {"after": "2026-10-13"}, {"focus_call": "unknown", "focus_trace": rows[0]["trace"]}]:
            assert get(base, "thread-calls", thread=key, **restriction)["rows"] == []
        for query in ["", "?thread="]:
            assert send(base, "/api/sidecar/thread-calls" + query, expected=None)[0] == 400
    with sqlite3.connect(db) as sql:
        sql.execute("DROP TRIGGER activity_document_delete")
        sql.execute("DELETE FROM activity_documents")
        sql.execute("UPDATE activity_index_progress SET cursor='' WHERE id=1")
        sql.execute("DELETE FROM conversation_calls WHERE call_id=?", (unthreaded["spanId"],))
        sql.execute("UPDATE conversation_calls SET preview=json_set(preview,'$.outputs',json('[]'),'$.output_count',0) WHERE call_id=?", (tool["spanId"],))
    with server(binary, db, *flags) as base:
        assert get(base, "thread-calls", thread=key, **focus) == second
        for _ in range(100):
            found = get(base, "activity-search", q="order writes rejected")
            if found["index_ready"]:
                break
            time.sleep(0.02)
        assert found["index_ready"] and [row["call"] for row in found["rows"]] == [tool["spanId"]]
        assert [row["call"] for row in get(base, "activity-search", q="shipping receipt")["rows"]] == [unthreaded["spanId"]]
    with sqlite3.connect(db) as sql:
        sql.execute("DROP TABLE conversation_calls")
        sql.execute("PRAGMA user_version=22")
    with server(binary, db, *flags) as base:
        assert get(base, "thread-calls", thread=key, **focus) == second
    with sqlite3.connect(db) as sql:
        sql.execute("UPDATE traces SET seen=0 WHERE id=?", (rows[17]["trace"],))
    with server(binary, db, *flags) as base:
        assert not any(row["call"] == tool["spanId"] for row in get(base, "activity-search", q="rejected")["rows"])
    with server(binary, folder / "private-conversation.sqlite") as base:
        send_traces(base, export(spans[0], tool))
        row, tool_row = get(base, "thread-calls", thread=key)["rows"]
        assert "Alternative declaration for page" not in json.dumps(row)
        assert not (row["dialogue"]["user"] or {}).get("text")
        assert "Order write rejected" not in json.dumps(tool_row)
        assert get(base, "activity-search", q="alternative declaration")["rows"] == []
        assert get(base, "activity-search", q="rejected")["rows"] == []


def check_tool_handoffs(binary, db):
    receiver = case_span("handoff", "undeclared")
    start = CLOCK * 10**9
    cases = ["same", "different", "missing", "duplicate", "late", "unknown"]
    receiver["attributes"] = [field for field in receiver["attributes"]
                              if field["key"] not in {"gen_ai.input.messages", "a2context.context.semantics"}]
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
            assert summary.pop("themes") == ([{"kind": "conflict", "conversations": 1, "groups": 1}]
                                             if impact["conversations"] else [])
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


def main():
    parser = argparse.ArgumentParser(description="Check the receiver with synthetic telemetry.")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sdk-python")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp)
        check_capture(args.binary.resolve(), folder, args.sdk_python)
        check_thread_calls(args.binary.resolve(), folder)
        check_tool_handoffs(args.binary.resolve(), folder / "tools.sqlite")
        check_conversation_population(args.binary.resolve(), folder / "population.sqlite")
        if args.sdk_python:
            check_first_recommendation(args.binary.resolve(), folder, args.sdk_python)


if __name__ == "__main__":
    main()
