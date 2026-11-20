"""Run the scored-input download, two-proposal CLI, and attached-result journey."""
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode

from sidecar_prefix_cohort import CASES, ORG, get, send, send_traces, server, span, trace_for


def run(binary, folder):
    cases = json.loads(CASES.read_text())
    case = cases[0]
    first = trace_for(case, 1, scores_only=True)
    second = trace_for(case, 2, scores_only=True)
    span(second)["traceId"] = "f" * 32
    actual = case["envelope"]["shadow"]
    # The adapter replays the saved proposals for these exact request bodies.
    runner = folder / "runner.py"
    runner.write_text("""import hashlib, json, pathlib, sys
payload = sys.stdin.buffer.read()
key = hashlib.sha256(payload).hexdigest()
with pathlib.Path(sys.argv[2]).open('a') as log:
    print(key, file=log)
print(json.dumps(json.loads(pathlib.Path(sys.argv[1]).read_text())[key]))
""")
    outputs = {actual["captured_request_sha256"]:actual["baseline"][0]["choice"]}
    outputs.update({e["request_sha256"]:e["trials"][0]["choice"] for e in actual["effects"]})
    choices = folder / "proposals.json"
    choices.write_text(json.dumps(outputs))
    log = folder / "calls.log"
    scored, checked = folder / "scored.json", folder / "checked.json"
    db = folder / "experiments.sqlite"
    with server(binary, db, "--capture-content") as base:
        send_traces(base, first)
        send_traces(base, second)
        group, = get(base, "prefix-groups", org=ORG)["rows"]
        assert (group["conversations"],group["needs_trial"],group["experiments"]) == (2,2,1)
        scope = {"design":group["design"],"org":ORG}
        cases_pending = get(base,"prefix-cases",**scope,needs_trial=1)["rows"]
        assert len(cases_pending) == 2
        locator = {**scope, **{k:cases_pending[0][k] for k in ("trace","call","report")}}
        pending = get(base,"prefix-input",**locator,needs_trial=1)
        assert "shadow" not in pending
        assert "gen_ai.output.messages" not in pending["request"]["attributes"]
        scored.write_text(json.dumps(pending))
        command = [binary,"test-prefix",scored,"fixture-runner",checked,"--",sys.executable,runner,choices,log]
        # Tampering with the original must fail before the runner receives any request.
        stale = deepcopy(pending)
        stale["candidate"]["captured_request_sha256"] = "0"*64
        scored.write_text(json.dumps(stale))
        assert subprocess.run(command, capture_output=True).returncode != 0
        assert not log.exists() and not checked.exists()
        # An unresolved selector must abstain even when its candidate report is valid.
        indirect = json.loads((CASES.parent.parent / "prefix-indirect/cases.json").read_text())
        ambiguous = next(r["envelope"] for r in indirect if r["id"] == "atlas-calendar-60-plants-plants")
        scored.write_text(json.dumps(ambiguous))
        assert subprocess.run(command,capture_output=True).returncode != 0
        assert not log.exists() and not checked.exists()
        scored.write_text(json.dumps(pending))
        subprocess.run(command, check=True, capture_output=True)
        report = json.loads(checked.read_text())["shadow"]
        assert log.read_text().splitlines() == [actual["captured_request_sha256"],actual["effects"][0]["request_sha256"]]
        assert len(report["baseline"]) == len(report["effects"]) == 1
        submit = {"scope":dict(locator,needs_trial="1",verdict="untested"),"shadow":report}
        route = "/api/prefix-results"
        send(base,route,{**submit, "scope":dict(locator,user="other")},expected=404)
        wrong = deepcopy(report)
        wrong["captured_request_sha256"] = "0"*64
        send(base,route,{**submit,"shadow":wrong},expected=400)
        wrong_arm = deepcopy(actual)
        wrong_arm["effects"] = wrong_arm["effects"][1:]
        send(base,route,{**submit,"shadow":wrong_arm},expected=400)
        assert get(base,"prefix-groups",**scope)["rows"][0]["needs_trial"] == 2
        send(base,route,submit,expected=201)
        send(base,route,submit)
        assert not get(base,"prefix-cases",**scope,needs_trial=1)["rows"]
        group, = get(base,"prefix-groups",**scope)["rows"]
        assert (group["needs_trial"], group["confirmed"], group["untested"]) == (0,1,1)
        duplicate = get(base,"prefix-cases",**scope,verdict="untested")["rows"][0]
        second_locator = {**scope, **{k:duplicate[k] for k in ("trace","call","report")}}
        other = get(base,"prefix-check",**second_locator)
        assert other["review"]["verdict"] == "untested" and other["matching_trial"]["trace"] == locator["trace"]
        detail = get(base,"prefix-check",**locator)["review"]
        assert detail["verdict"] == "confirmed" and detail["runs"][0]["origin"] == "sidecar.prefix_trials"
        # A newly arrived raw span and exporter retries preserve the attachment.
        send_traces(base, second)
        parent = deepcopy(span(second))
        parent.update(spanId="c"*16,name="Background work",attributes=[])
        late = deepcopy(second)
        late["resourceSpans"][0]["scopeSpans"][0]["spans"] = [parent]
        send_traces(base,late)
        assert get(base,"prefix-check",**locator)["review"] == detail
        conflict = deepcopy(report)
        conflict["id"] = actual["id"]
        conflict["effects"] = deepcopy(actual["effects"][:1])
        conflict["baseline_summary"] = deepcopy(actual["baseline_summary"])
        conflict["expected"] = deepcopy(actual["expected"])
        conflict["match_rule"] = deepcopy(actual["match_rule"])
        effect = conflict["effects"][0]
        effect["trials"] = deepcopy(actual["baseline"])
        effect["summary"] = deepcopy(actual["baseline_summary"])
        effect["difference_pp"] = 0.0
        conflict["id"] = report["id"]
        send(base,route,{**submit,"shadow":conflict},expected=409)
        conflict["runner_id"] = "different runner"
        send(base,route,{**submit,"shadow":conflict},expected=201)
        assert get(base,"prefix-check",**locator)["review"]["verdict"] == "mixed"
    with sqlite3.connect(db) as conn:
        conn.execute("UPDATE prefix_checks SET choices=json_remove(choices,'$.trial_key')")
        conn.execute("PRAGMA user_version=27")
    with server(binary,db,"--capture-content") as base:
        assert get(base,"prefix-check",**locator)["review"]["verdict"] == "mixed"
        assert get(base,"prefix-groups",**scope)["rows"][0]["needs_trial"] == 0
    with server(binary,folder/"redacted.sqlite") as base:
        send_traces(base,first)
        send(base,"/api/sidecar/prefix-input?" + urlencode(locator),expected=404)
        send(base,route,submit,expected=403)
    with sqlite3.connect(db) as conn:
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute("DELETE FROM traces")
        assert conn.execute("SELECT count(*) FROM prefix_trials").fetchone() == (0,)


if __name__ == "__main__":
    with TemporaryDirectory() as tmp:
        run(Path(sys.argv[1]).resolve(),Path(tmp))
    print("Verified two calls, scoped uploads, duplicate suppression, contradictory trials, migration and retention.")
