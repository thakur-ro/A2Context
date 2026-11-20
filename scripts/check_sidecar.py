"""Exercise the sidecar API, rank labeled cases, and reproduce a bounded load test."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from copy import deepcopy
from datetime import datetime, timezone
from hashlib import sha256
import json
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
    with tempfile.TemporaryFile() as logs:
        process = subprocess.Popen(
            [str(binary), "sidecar", "--db", str(db), "--listen", f"127.0.0.1:{port}", *flags],
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


def check_replay(base, send_trace=send_traces):
    sys.path.insert(0, str(ROOT / "examples"))
    from send_otlp_replay import prepare as prepare_replay

    trace, report = prepare_replay("held-out-experiment")
    path = "/api/replay-reports"
    send_trace(base, trace)
    tampered = deepcopy(report)
    tampered["baseline_request_sha256"] = "0" * 64
    send(base, path, tampered, expected=400)
    send(base, path, report, expected=201)
    send(base, path, report)
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


def check_order_tests(base):
    fixture = json.loads((ROOT / "examples/redundant-routing/measured/local-order.json").read_text())
    reports = []
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
            send(base, "/api/replay-reports", report, expected=201)
            reports.append(report)
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
    repeated["attributes"]["nexusweave.request.body"] = fixture["runs"][0]["calls"][0]["recorded_request"]["body"]
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


def check_predecision_recommendations(base):
    fixture = ROOT / "examples/runtime-comparison/pending-routing"
    read = lambda name: json.loads((fixture / name).read_text())
    request, candidate, shadow = read("request.json"), read("report.json"), read("shadow-report.json")
    preview_path = "/api/sidecar/preview"
    before = get(base, "health")["coverage"]
    recommendation = send(base, preview_path, {"request": request, "candidate": candidate, "shadow": shadow})[1]
    result = recommendation["recommendations"]
    assert not recommendation["unverified_probes"]
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
    for _ in range(10):
        start = time.monotonic()
        impact = get(base, "impact", org=org)
        factors = get(base, "factors", org=org)
        reads.append(1000 * (time.monotonic() - start))
        assert impact == {"conversations": batch_size, "changed": 0, "unlinked_requests": 0, "factors": batches,
                          "evidence_counts": {"proposal": 0, "rate": 0, "score": 0, "tested": 0, "inputs": batch_size},
                          "replay_outcomes": {"mixed": 0, "changed_only": 0, "equal_only": 0, "unmeasured": batch_size}}
        assert len(factors["rows"]) == min(batches, 15)
    return {"responses": {code: sum(status == code for status, _ in outcomes) for code in codes},
            "wall_s": round(elapsed, 3), "p95_successful_export_ms": round(timings[int((len(timings) - 1) * 0.95)], 2),
            "last_processing_lag_ms": final["processing_lag_ms"], "batch_size": batch_size,
            "clients": clients, "extra_bytes_per_span": payload_bytes, "retry_recovered_batches": recovered,
            "retry_wall_s": round(recovery_seconds, 3), "retained_traces": coverage["traces"],
            "overview_read_pairs": len(reads), "median_sequential_overview_ms": round(median(reads), 2)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/nexusweave")
    parser.add_argument("--sdk-python", help="Interpreter with OpenTelemetry SDK and OTLP/HTTP exporter installed")
    parser.add_argument("--overload-url", help="Run load only against an isolated persistent sidecar")
    parser.add_argument("--clients", type=int, default=24)
    parser.add_argument("--batches", type=int, default=192)
    parser.add_argument("--batch-size", type=int, default=12)
    parser.add_argument("--payload-bytes", type=int, default=4096)
    args = parser.parse_args()
    if args.overload_url:
        print(json.dumps(load(args.overload_url, args.clients, args.batches, args.batch_size, args.payload_bytes), indent=2))
        return
    with tempfile.TemporaryDirectory() as directory:
        folder = Path(directory)
        check_capture(args.binary, folder, args.sdk_python)
        check_external_batch(args.binary, folder)
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
            check_replacement_comparison(base)
            check_replay_variation(base)
            check_order_tests(base)
            pairs = get(base, "order-tests", org="order-check")
            proposal_rows = get(base, "conversation-outcomes", level=1)
            assert proposal_rows["rows"]
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
