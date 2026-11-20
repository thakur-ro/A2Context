"""Compare every graded source edit against the frozen activity study."""
import argparse
from collections import Counter
from copy import deepcopy
import json
from pathlib import Path
import re
import sqlite3
import subprocess
from tempfile import TemporaryDirectory
from urllib.parse import urlencode

from sidecar_prefix_cohort import CASES, get, send, send_traces, server, span, trace_for

ACTIVITIES = CASES.parent.parent / "prefix-indirect/cases.json"


def grade(trials, reference):
    assert len(trials) == 1  # The frozen study used one greedy generation per arm.
    answer = trials[0]["choice"]["outcome"]
    numbers = re.findall(r"\b[0-9]+\b",answer)
    if len(numbers) == 1 and numbers[0] in ("30","60"):
        return numbers[0] == reference
    return None


def expected(before, after):
    if before is None or after is None:
        return "unresolved"
    if before:
        return "kept" if after else "lost"
    return "restored" if after else "missed"


def check(binary, folder, cases):
    db = folder / "references.sqlite"
    org = "labeled-activities"
    with server(binary, db, "--capture-content") as base:
        for i, case in enumerate(cases,1):
            trace = trace_for(case,i,org=org,reference=True)
            send_traces(base,trace)
            send_traces(base,trace)
        group, = get(base,"prefix-groups",org=org)["rows"]
        scope = {"org":org,"design":group["design"]}
        assert (group["referenced"],group["restored"],group["lost"],group["forecast_restore"]) == (16,4,7,0)
        rows = get(base,"prefix-references",**scope)["rows"]
        assert sum(r["conversations"] for r in rows if r["outcome"]=="unselected")==5
        effects = Counter()
        for i,case in enumerate(cases,1):
            record = span(trace_for(case,i,org=org,reference=True))
            locator = dict(scope,trace="otlp:"+record["traceId"],call=record["spanId"],report="room-duration")
            detail = get(base,"prefix-check",**locator)["review"]
            target = str(case["minutes"][case["requested"]])
            ref = detail["reference"]
            assert ref["continuation"] == target and ref["set"] == "original-booking-duration-v1"
            shadow = case["envelope"]["shadow"]
            before = grade(shadow["baseline"],target)
            edits = [expected(before,grade(e["trials"],target)) for e in shadow["effects"]]
            assert ref["edits"] == edits
            if len(detail["reversals"]) == 1:
                selected = detail["reversals"][0]-1
                assert ref["outcome"] == edits[selected]
                if ref["outcome"] == "restored":
                    assert shadow["effects"][selected]["source_id"] != case["requested"]
                effects.update([ref["outcome"]])
            else:
                assert ref["outcome"]=="unselected" and ref["prediction"] is None
        assert effects == {"restored":4,"lost":7}
        for row in rows:
            args = {"source":row["source"]} if row["source"] else {"unnamed":"1"}
            filters = dict(scope,ref_set=row["reference_set"],ref_outcome=row["outcome"],**args)
            samples = get(base,"prefix-cases",**filters)["rows"]
            assert len(samples) == row["conversations"]
            for witness in samples:
                location = {k:witness[k] for k in ("trace","call","report")}
                send(base,"/api/sidecar/prefix-check?" + urlencode(dict(filters,**location,ref_set="unrelated")),expected=404)
        # The label belongs to the original task. Withhold it and the same measured
        # switch remains visible without being counted as a restored or lost match.
        for trace_org, reference in [("unlabeled",False),("forecast",True)]:
            for i,case in enumerate(cases,1):
                send_traces(base,trace_for(case,i,org=trace_org,reference=reference,scores_only=reference))
            g, = get(base,"prefix-groups",org=trace_org)["rows"]
            assert (g["restored"],g["lost"]) == (0,0)
            if reference:
                assert (g["forecast_restore"],g["needs_trial"]) == (4,11)
                s=dict(org=trace_org,design=g["design"],needs_trial="1",ref_prediction="restored")
                witness,*_ = get(base,"prefix-cases",**s)["rows"]
                where=dict(s,**{k:witness[k] for k in ("trace","call","report")})
                envelope = get(base,"prefix-input",**where)
                assert "shadow" not in envelope and envelope["candidate"]["reference_target"]["candidate_id"] == "60"
                case = cases[int(witness["thread"].split()[-1])-1]
                send(base,"/api/prefix-results",{"scope":where,"shadow":case["envelope"]["shadow"]},expected=201)
                g,=get(base,"prefix-groups",org=trace_org)["rows"]
                assert (g["forecast_restore"],g["restored"])==(3,1)
            else:
                assert g["confirmed"] == 11 and g["referenced"] == 0
                assert not get(base,"prefix-cases",org=trace_org,design=g["design"],reference="1")["rows"]
    with sqlite3.connect(db) as conn:
        conn.execute("UPDATE prefix_checks SET choices=json_remove(choices,'$.reference_set','$.reference_outcome','$.reference_prediction')")
        conn.execute("PRAGMA user_version=28")
    with server(binary,db,"--capture-content") as base:
        assert get(base,"prefix-groups",**scope)["rows"] == [group]
    with server(binary, folder / "redacted.sqlite") as base:
        send_traces(base, trace_for(cases[0], 1, org=org, reference=True))
        assert get(base, "prefix-groups", org=org)["rows"] == []
        assert get(base, "prefix-references", **scope)["rows"] == []
    # Invalid or conflicting evaluation metadata cannot create a new model forecast.
    envelope=deepcopy(cases[-1]["envelope"])
    candidate=envelope["candidate"]
    def inspect():
        return json.loads(subprocess.check_output([binary,"inspect-pending","-"],input=json.dumps(envelope),text=True))["prefix_reviews"][0]
    original=inspect()
    for target, result in [("60","restored"),("30","lost"),("unknown",None)]:
        candidate["reference_target"]={"set":"fixture","candidate_id":target}
        review=inspect()
        assert review["reversals"] == original["reversals"] and review["runs"] == original["runs"]
        if result:
            assert review["reference"]["outcome"] == result
        else:
            assert "issue" in review["reference"] and "outcome" not in review["reference"]


def measure(binary, folder, cases):
    from sidecar_conflict_coverage import time_reads
    with server(binary, folder/"measure.sqlite", "--capture-content") as base:
        for i in range(128):
            trace = trace_for(cases[i%16],i+1,reference=True)
            span(trace)["traceId"]=f"{i+1:032x}"
            send_traces(base,trace)
        g, = get(base,"prefix-groups")["rows"]
        assert (g["restored"],g["lost"],g["inputs"])==(32,56,16)
        for route in ["prefix-groups","prefix-references","prefix-cases"]:
            params={"design":g["design"]}
            if route!="prefix-groups":
                params["ref_outcome"]="restored"
            time_reads(base,route,params)


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary",type=Path)
    parser.add_argument("--measure",action="store_true")
    args=parser.parse_args()
    with TemporaryDirectory() as tmp:
        (measure if args.measure else check)(args.binary.resolve(),Path(tmp),json.loads(ACTIVITIES.read_text()))
    print("Verified reference outcomes on labeled activity runs.")
