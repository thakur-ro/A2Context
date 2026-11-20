"""Verify required-source receipts across real preflight, OTLP and the findings store."""
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from check_sidecar import attrs, case_span, export, get, root_span, send_traces, server


def checked_span(binary, index, *, anchored=False, tamper=None, user="pat", source_version="v1"):
    request = json.loads((ROOT / "examples/exchange-approval/pending.json").read_text())
    fields = request["attributes"]
    fields["nexusweave.context.semantics"]["request_sources"][0]["source_version"] = source_version
    if not anchored:
        fields["nexusweave.context.semantics"]["request_sources"].pop()
    if index % 2:
        fields["nexusweave.required_sources"]["any_of"].reverse()
    report = json.loads(subprocess.check_output([binary, "preflight", "-"], input=json.dumps(request), text=True))
    assert report["source_coverage"]["matched"] == anchored
    receipt = report["receipt"]
    assert receipt["version"] == 2
    if tamper == "contract":
        receipt["required_sources"]["name"] += " (modified)"
    if tamper == "anchor":
        fields["nexusweave.context.semantics"]["request_sources"][0]["source_version"] = "different"
    if tamper == "body":
        fields["nexusweave.request.body"] += " "
    fields.pop("nexusweave.required_sources")
    fields.pop("service.name")
    fields.update({"nexusweave.preflight":receipt, "gen_ai.operation.name":"chat",
                   "gen_ai.conversation.id":f"checkout-{index}", "nexusweave.user.id":user})
    span = case_span("source-coverage", "undeclared", index)
    span["attributes"] = attrs({k:v if isinstance(v,str) else json.dumps(v) for k,v in fields.items()})
    return span


def verify(binary):
    with TemporaryDirectory() as tmp:
        path = Path(tmp) / "captured.sqlite"
        gaps = [checked_span(binary, i) for i in (0, 1)]
        batch = export(*gaps)
        with server(binary, path, "--capture-content") as base:
            send_traces(base, batch)
            send_traces(base, export(root_span(gaps[0])))
            send_traces(base, batch)
            others = [checked_span(binary, 2, anchored=True), checked_span(binary, 3, tamper="contract"),
                      checked_span(binary, 4, tamper="anchor"), checked_span(binary, 5, tamper="body")]
            send_traces(base, export(*others))
            send_traces(base, export(checked_span(binary, 6, user="lee")))
            isolated = export(checked_span(binary, 7))
            isolated["resourceSpans"][0]["resource"]["attributes"] = attrs({"service.name":"other-workflow"})
            send_traces(base, isolated)
            pat, = get(base, "findings", kind="source_gap", user="pat", workflow="policy-workflow")["rows"]
            assert (pat["threads"], pat["occurrences"]) == (2, 2), pat
            detail = pat["evidence"]["detail"]
            assert detail["coverage"]["matched"] is False
            assert {s["source"] for s in detail["sources"]} == {"confirmation-cache"}
            assert len(get(base, "findings", kind="source_gap")["rows"]) == 2
            factor, = get(base, "factors", user="pat", workflow="policy-workflow", kind="source_gap")["rows"]
            assert factor["signal"] == "source_gap" and factor["id"] == detail["factor"]
            assert get(base, "findings", kind="source_gap", user="lee")["rows"][0]["threads"] == 1
            assert get(base, "findings", terms="verified-consent-event", user="pat")["rows"]
            events = get(base, "occurrences", group=pat["id"], user="pat")["rows"]
            assert {r["trace"] for r in events} == {"otlp:" + s["traceId"] for s in gaps}
            send_traces(base, export(checked_span(binary, 8, source_version="v2")))
            distinct = get(base, "findings", kind="source_gap", user="pat", workflow="policy-workflow")["rows"]
            assert {row["threads"] for row in distinct} == {1, 2}
        with server(binary, path, "--capture-content") as base:
            assert get(base, "findings", group=pat["id"], user="pat")["rows"][0]["threads"] == 2
        with server(binary, Path(tmp) / "redacted.sqlite") as base:
            send_traces(base, batch)
            assert not get(base, "findings", kind="source_gap")["rows"]
            trace = get(base, "trace", id="otlp:" + gaps[0]["traceId"])
            assert "verified-consent-event" not in json.dumps(trace)
            assert trace["trace"]["calls"][0].get("preflight_receipt") is None
    print("Verified scoped source gaps, exact evidence, deduplication, restart, redaction and mismatched receipts.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
