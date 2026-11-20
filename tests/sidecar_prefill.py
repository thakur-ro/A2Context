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
from prefill_cohort import ATTRIBUTE, export, inputs


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


def check_routing(binary, record, probe, folder):
    request = {"version": 1, "attributes": {**inputs(record), ATTRIBUTE: deepcopy(probe)}}
    raw = request["attributes"][ATTRIBUTE]["routing_test"]

    def inspect():
        out = subprocess.run([binary, "inspect-pending", "-"], check=True, input=json.dumps({"request": request}),
                             capture_output=True, text=True)
        return json.loads(out.stdout)["prefill"]

    raw["cells"][2][1]["request_sha256"] = raw["cells"][0][1]["request_sha256"]
    bad = inspect()
    assert "routing_test" not in bad and "reconstructed request" in bad["routing_issue"] and bad["ranked"]
    raw["cells"] = deepcopy(probe["routing_test"]["cells"])
    raw["cells"][1][0]["trial"] = {"status": "failed"}
    measured = inspect()["routing_test"]
    assert measured["pattern"] == "incomplete" and measured["cells"][1]["changed"] is None
    raw["cells"] = deepcopy(probe["routing_test"]["cells"])
    raw["cells"][0][1]["trial"]["choice"]["label"] = "unlisted"
    assert inspect()["routing_test"]["cells"][0]["trials"][1]["listed"] is False

    for labels, expected in [(["stock", "stock", "price", "price"], "instruction"),
                             (["stock"] * 4, "constant"), (["stock", "stock", "stock", "price"], "combined")]:
        for cell, label in zip(raw["cells"], labels):
            cell[1]["trial"]["choice"]["label"] = label
        assert inspect()["routing_test"]["pattern"] == expected

    del request["attributes"][ATTRIBUTE]["routing_test"]
    req, output = folder / "scored.json", folder / "checked.json"
    req.write_text(json.dumps(request))
    adapter = folder / "runner.py"
    adapter.write_text("import json,sys\n"
                       "r=json.load(sys.stdin)\n"
                       "print(json.dumps(dict(kind='model_output',label='answer',outcome=r['messages'][1]['content'])))\n")
    command = [binary, "test-routing", req, FOLDER / "routing-design.json", "test-runner", output, "--", sys.executable, adapter]
    subprocess.run(command, check=True)
    sample = json.loads(output.read_text())["attributes"][ATTRIBUTE]["routing_test"]
    assert [arm["request_sha256"] for cell in sample["cells"] for arm in cell] == [
        arm["request_sha256"] for cell in probe["routing_test"]["cells"] for arm in cell]
    assert len({arm["request_sha256"] for cell in sample["cells"] for arm in cell}) == 8
    assert subprocess.run(command, capture_output=True).returncode != 0


def check_recurrence(binary, folder, sdk_python):
    saved = json.loads((FOLDER / "routing-holdout-results.json").read_text())
    reports = {r["id"]: r["extra"]["metadata"][ATTRIBUTE] for r in saved}
    payload = export(saved, reports)
    archive = folder / "replication"
    with server(binary, archive, "--capture-content") as base:
        send_traces(base, payload)
        if sdk_python:
            sdk_sender(sdk_python)(base, payload)
        card, = recommendation(base, org="retail-south")
        profiles = card["routing_profiles"]
        assert [(r["signature"],r["tool_signature"],r["inputs"]) for r in profiles] == [
            ("0000","0000",2), ("0101","0001",1), ("0101","0101",1)]
        assert card["routing_conversations"] == 4
        for row in profiles:
            one, = recommendation(base, group=card["id"], **row["example"])
            assert one["routing_profiles"] == [{**row,"inputs":1}]
            assert one["routing_conversations"] == 1
        assert not recommendation(base, org="other")
    with sqlite3.connect(archive) as connection:
        connection.execute("UPDATE observations SET detail=json_remove(detail,'$.routing_test.design') WHERE kind='prefill'")
        connection.execute("PRAGMA user_version=10")
    with server(binary, archive, "--capture-content") as base:
        assert recommendation(base, org="retail-south") == [card]
        def repeat(probe, attrs):
            attrs["gen_ai.conversation.id"]["stringValue"] = "another-conversation"
        send_traces(base, modified(payload, 801, repeat))
        after, = recommendation(base)
        assert [r["inputs"] for r in after["routing_profiles"]] == [2,1,1] and after["routing_conversations"] == 5
        def disagree(probe, attrs):
            probe["routing_test"]["cells"][1][1]["trial"] = probe["routing_test"]["cells"][1][0]["trial"]
        send_traces(base, modified(payload, 802, disagree))
        mixed, = recommendation(base)
        assert mixed["action"]["title"] == "Check the disagreeing repeat trials"
        assert any(r["signature"] == "mixed" and r["inputs"] == 1 for r in mixed["routing_profiles"])
        def another_runner(probe, attrs):
            probe["routing_test"]["runner_id"] = "different-decoding"
        send_traces(base, modified(payload, 803, another_runner))
        other, = recommendation(base, trace=f"otlp:{803:032x}")
        assert other["evidence"]["detail"]["routing_test"]["design"] != card["evidence"]["detail"]["routing_test"]["design"]
        assert sum(r["inputs"] for r in other["routing_profiles"]) == 1
        preferred, = recommendation(base)
        assert preferred["routing_profiles"] == mixed["routing_profiles"] and preferred["routing_design_count"] == 2
        alternate, = recommendation(base, routing_design=other["evidence"]["detail"]["routing_test"]["design"])
        assert alternate["routing_profiles"] == other["routing_profiles"]
    with server(binary, folder/"partial-routing", "--capture-content") as base:
        partial = deepcopy(reports)
        del partial[saved[0]["id"]]["routing_test"]
        partial[saved[1]["id"]]["routing_test"]["cells"][0][1]["trial"] = {"status": "failed"}
        send_traces(base, export(saved, partial))
        result, = recommendation(base)
        assert result["action"]["title"] == "Complete the missing routing trials"
        assert sum(r["inputs"] for r in result["routing_profiles"]) == 3 and result["routing_conversations"] == 3
        assert sum(r["inputs"] for r in result["prefill_counts"]) == 4
        assert any("?" in r["signature"] for r in result["routing_profiles"])



def check(binary, folder, sdk_python):
    records = json.loads((FOLDER / "shadow-cohort.json").read_text())
    probes = json.loads((FOLDER / "prefill-cohort.json").read_text())
    crossed, = [r for r in records if "routing_test" in probes[r["id"]]]
    check_routing(binary, crossed, probes[crossed["id"]], folder)
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
        unchanged, = [r for r in south if r["action"]["title"] == "Retest the tool inventory on another request"]
        routing = unchanged["evidence"]["detail"]["routing_test"]
        assert routing["pattern"] == "tools"
        assert [cell["changed"] for cell in routing["cells"]] == [False, True, False, True]
        assert [[t["label"] for t in cell["trials"]] for cell in routing["cells"]] == [
            ["stock", "stock"], ["stock", "price"], ["stock", "stock"], ["stock", "price"]]
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
    with sqlite3.connect(db) as connection:
        connection.execute("UPDATE traces SET run=json_remove(run,'$.calls[0].prefill_probe.report.routing_test')")
        connection.execute("UPDATE observations SET detail=json_remove(detail,'$.routing_test') WHERE kind='prefill'")
        connection.execute("PRAGMA user_version=9")
    with server(binary, db, "--capture-content") as base:
        assert recommendation(base, org="retail-south") == south
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
        check_recurrence(args.binary.resolve(), Path(folder), args.sdk_python)
    print("Verified recorded prefill coverage, ties, controlled routing trials, scope, retries, redaction, distinct-input routing profiles, and v8/v9/v10 recovery.")
