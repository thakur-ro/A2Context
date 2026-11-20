"""Check scope, repeat denominators, proposal bindings and v8 archive recovery."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ROOT / "examples/runtime-comparison/pending-routing"
sys.path[:0] = [str(ROOT / "scripts"), str(FOLDER)]
from check_sidecar import get, sdk_sender, send_traces, server
from prefill_cohort import ATTRIBUTE, export


def recommendation(base, **scope):
    return get(base, "findings", kind="prefill", user="routing-demo", **scope)["rows"]


def modified(doc, identifier, update):
    record = deepcopy(doc)
    spans = record["resourceSpans"][0]["scopeSpans"][0]["spans"]
    del spans[1:]
    span = spans[0]
    span["traceId"], span["spanId"] = f"{identifier:032x}", f"{identifier:016x}"
    attrs = {row["key"]: row["value"] for row in span["attributes"]}
    probe = json.loads(attrs[ATTRIBUTE]["stringValue"])
    update(probe, attrs)
    attrs[ATTRIBUTE]["stringValue"] = json.dumps(probe)
    span["attributes"] = [{"key": k, "value": v} for k, v in attrs.items()]
    return record


def check(binary, folder, sdk_python):
    records = json.loads((FOLDER / "shadow-cohort.json").read_text())
    probes = json.loads((FOLDER / "prefill-cohort.json").read_text())
    doc = export(records, probes)
    db = folder / "archive"
    with server(binary, db, "--capture-content") as base:
        send_traces(base, doc)
        send_traces(base, doc)
        north, = recommendation(base, org="retail-north")
        assert north["action"]["title"] == "Repeat the leading edit on a fresh request"
        assert north["threads"] == north["occurrences"] == 2
        leader, = north["prefill_counts"]
        assert leader["inputs"] == 2 and leader["leader"]["source"] == "request-type"
        follow, = north["prefill_followup"]
        assert (follow["changed"], follow["equal"], follow["mixed"]) == (2, 0, 0)
        south = recommendation(base, org="retail-south")
        unchanged, = [r for r in south if r["action"]["title"] == "Check a later output token"]
        assert unchanged["evidence"]["detail"]["tool_names"] == ["stock"]
        assert len(south) == 2 and {row["prefill_followup"][0]["changed"] for row in south} == {0, 1}
        assert get(base, "findings", kind="prefill", user="unrelated")["rows"] == []
        def failed(probe, attrs):
            probe["scores"]["baseline_request_sha256"] = "0" * 64
        send_traces(base, modified(doc, 701, failed))
        invalid = get(base, "trace", id=f"otlp:{701:032x}")
        assert len(invalid["prefill_issues"]) == 1 and all(f["kind"] != "prefill" for f in invalid["findings"])
        send_traces(base, modified(doc, 702, lambda probe, attrs: None))
        north, = recommendation(base, org="retail-north")
        assert north["prefill_counts"] == [leader] and north["prefill_followup"] == [follow]
        assert north["occurrences"] == 3
        def tie(probe, attrs):
            probe["scores"]["effects"][1]["js_bits"] = probe["scores"]["effects"][0]["js_bits"]
        send_traces(base, modified(doc, 703, tie))
        north, = recommendation(base, org="retail-north")
        assert sorted(row["leader"]["status"] for row in north["prefill_counts"]) == ["mixed", "unique"]
        assert north["prefill_followup"][0]["changed"] == 1
        def zero(probe, attrs):
            attrs["nexusweave.organization.id"]["stringValue"] = "zero"
            for row in probe["scores"]["effects"]:
                row["js_bits"] = 0
        send_traces(base, modified(doc, 704, zero))
        z, = recommendation(base, org="zero")
        assert z["prefill_counts"] == [{"leader":{"status":"zero"}, "inputs":1}] and z["prefill_followup"] == []
        def bad_shadow(probe, attrs):
            attrs["nexusweave.organization.id"]["stringValue"] = "unpaired"
            reports = json.loads(attrs["nexusweave.shadow_probes"]["stringValue"])
            reports[0]["effects"][0]["request_sha256"] = "0"*64
            attrs["nexusweave.shadow_probes"]["stringValue"] = json.dumps(reports)
        send_traces(base, modified(doc, 705, bad_shadow))
        unpaired, = recommendation(base, org="unpaired")
        assert unpaired["prefill_counts"][0]["leader"]["status"] == "unique" and unpaired["prefill_followup"] == []
    with sqlite3.connect(db) as connection:
        connection.execute("UPDATE traces SET run=json_remove(run,'$.calls[0].prefill_probe')")
        connection.execute("DELETE FROM observations WHERE kind='prefill'")
        connection.execute("PRAGMA user_version=8")
    with server(binary, db, "--capture-content") as base:
        restored, = recommendation(base, org="retail-north")
        assert restored == north
    if sdk_python:
        with server(binary, folder/"sdk", "--capture-content") as base:
            sdk_sender(sdk_python)(base, doc)
            sdk, = recommendation(base, org="retail-north")
            assert sdk["prefill_counts"] == [leader] and sdk["prefill_followup"] == [follow]
            assert recommendation(base, org="retail-south") == south
    with server(binary, folder/"redacted") as base:
        send_traces(base, doc)
        assert recommendation(base, org="retail-north") == []
    with sqlite3.connect(folder/"redacted") as connection:
        assert all(ATTRIBUTE not in raw for (raw,) in connection.execute("SELECT raw FROM traces"))
    for record in records:
        record["extra"]["metadata"][ATTRIBUTE] = probes[record["id"]]
    (folder/"langsmith.json").write_text(json.dumps(records))
    subprocess.run([binary,"import-langsmith",folder/"langsmith.json",folder/"evidence.json"], check=True)
    runs = json.loads((folder/"evidence.json").read_text())
    assert sum(c.get("prefill_probe",{}).get("report") is not None for r in runs for c in r["calls"]) == 4


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sdk-python", help="Also emit the corpus through the OpenTelemetry SDK exporter.")
    args = parser.parse_args()
    with TemporaryDirectory() as folder:
        check(args.binary.resolve(), Path(folder), args.sdk_python)
    print("Verified recorded prefill coverage, ties, exact proposal checks, scope, retries, redaction, and v8 recovery.")
