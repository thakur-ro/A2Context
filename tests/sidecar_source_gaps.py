"""Verify required-source receipts across real preflight, OTLP and the findings store."""
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from check_sidecar import attrs, case_span, export, get, root_span, send_traces, server


def checked_span(binary, index, *, anchored=False, tamper=None, user="pat", source_version="v1", expected=None):
    request = json.loads((ROOT / "examples/exchange-approval/pending.json").read_text())
    fields = request["attributes"]
    fields["nexusweave.context.semantics"]["request_sources"][0]["source_version"] = source_version
    if not anchored:
        fields["nexusweave.context.semantics"]["request_sources"].pop()
    if expected:
        fields["nexusweave.required_sources"]["expected_text_sha256"] = {"latest-customer-message": expected}
    if index % 2:
        fields["nexusweave.required_sources"]["any_of"].reverse()
    report = json.loads(subprocess.check_output([binary, "preflight", "-"], input=json.dumps(request), text=True))
    assert report["source_coverage"]["matched"] == (anchored and expected is None)
    receipt = report["receipt"]
    assert receipt["version"] == (3 if expected else 2)
    if tamper == "digest":
        receipt["required_sources"]["expected_text_sha256"]["latest-customer-message"] = "0" * 64
    if tamper == "downgrade":
        receipt["version"] = 2
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
            # Expected values are customer-specific. Group by the failing source and
            # preserve each occurrence's own digest and failure mode in its evidence.
            cases = [checked_span(binary, i, anchored=present, expected=digit * 64, tamper=tamper, user="sam")
                     for i, present, digit, tamper in [(9, True, "a", None), (10, True, "b", None),
                                                      (11, False, "c", None), (12, True, "d", "digest"),
                                                      (13, True, "e", "downgrade")]]
            send_traces(base, export(*cases))
            pinned = get(base, "findings", kind="source_gap", user="sam")["rows"]
            changed, missing = sorted(pinned, key=lambda row: -row["occurrences"])
            assert (changed["occurrences"], missing["occurrences"]) == (2, 1)
            assert changed["evidence"]["detail"]["coverage"]["text_mismatches"] == ["latest-customer-message"]
            mismatch, = [s for s in changed["evidence"]["detail"]["sources"] if "request_fragment" in s]
            assert mismatch["request_fragment"]["pointer"] == "/messages/3/content"
            assert mismatch["captured_text_sha256"] == sha256(
                b"Please pause the puzzle exchange. Keep the boots exchange as it is."
            ).hexdigest()
            assert changed["action"]["title"] == "Compare captured text with the upstream source"
            assert "text_mismatches" not in missing["evidence"]["detail"]["coverage"]
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
