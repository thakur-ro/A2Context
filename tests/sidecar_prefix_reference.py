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


def check_reference_pairs(base, scope):
    pairs = get(base, "prefix-reference-pairs", **scope)["rows"]
    expected = {"atlas-calendar": ["Booking 3", "Booking 4"], "birch-calendar": ["Booking 13", "Booking 14"]}
    assert {pair["source"]: [r["thread"] for r in pair["examples"]] for pair in pairs} == expected
    for pair in pairs:
        restored, lost = pair["examples"]
        assert [restored["outcome"], lost["outcome"]] == ["restored", "lost"]
        assert [restored["expected"], lost["expected"]] == ["60", "30"]
        for row in pair["examples"]:
            where = dict(scope, **{key: row[key] for key in ("trace", "call", "report")},
                         ref_set=pair["reference_set"], ref_edit=pair["edit"], pair_setup=pair["setup"],
                         edit_index=row["index"], edit_outcome=row["outcome"])
            checked = get(base, "prefix-check", **where)["review"]
            assert checked["conditions"][row["index"]]["label"] == pair["source"]
            assert checked["reference"]["edits"][row["index"] - 1] == row["outcome"]
            trials = checked["runs"][0]["rows"]
            for index, label in [(0, "original"), (row["index"], "edited")]:
                assert trials[index]["example"] == checked["prefix"] + row[label]
            for field, value in [("ref_edit", "other"), ("edit_index", "0"), ("pair_setup", "other"),
                                 ("edit_outcome", "kept"), ("ref_set", "other"), ("user", "other")]:
                send(base, "/api/sidecar/prefix-check?" + urlencode(dict(where, **{field: value})), expected=404)
    assert not get(base, "prefix-reference-pairs", **scope, user="other")["rows"]


def check_pair_controls(base, cases):
    def publish(org, case, index, set_name=None, flip_target=False):
        trace = trace_for(case, index, org=org, reference=True)
        record = span(trace)
        record["traceId"] = f"{index:032x}"
        field = next(a["value"] for a in record["attributes"] if a["key"] == "nexusweave.candidate_probes")
        reports = json.loads(field["stringValue"])
        target = reports[0]["reference_target"]
        if set_name:
            target["set"] = set_name
        if flip_target:
            target["candidate_id"] = "60" if target["candidate_id"] == "30" else "30"
        field["stringValue"] = json.dumps(reports)
        send_traces(base, trace)

    def pairs(org):
        group, = get(base, "prefix-groups", org=org)["rows"]
        return get(base, "prefix-reference-pairs", org=org, design=group["design"])["rows"]

    org = "paired-controls"
    left, right = cases[12:14]
    publish(org, left, 101)
    publish(org, right, 102, set_name="other-reference")
    assert not pairs(org)
    different_runner = deepcopy(right)
    different_runner["envelope"]["shadow"]["runner_id"] += "-other"
    publish(org, different_runner, 103)
    assert not pairs(org)
    different_version = deepcopy(right)
    envelope = different_version["envelope"]
    source = envelope["request"]["attributes"]["nexusweave.context.semantics"]["request_sources"][0]
    source["source_version"] += "-revised"
    for report in (envelope["candidate"], envelope["shadow"]):
        report["effects"][0]["source_version"] = source["source_version"]
    publish(org, different_version, 106)
    group, = get(base, "prefix-groups", org=org)["rows"]
    versioned, = get(base, "prefix-cases", org=org, design=group["design"],
                     trace="otlp:" + f"{106:032x}", ref_outcome="lost")["rows"]
    assert versioned["thread"] == "Booking 106"
    assert not pairs(org)
    # A tested edit can supply the comparison even when the scorer prefers
    # the same continuation under every edit. These scores are a test fixture.
    abstention = deepcopy(right)
    candidate = abstention["envelope"]["candidate"]
    for condition, effect in zip(candidate["scores"]["conditions"][1:], candidate["effects"]):
        condition["log_probs"] = candidate["scores"]["conditions"][0]["log_probs"]
        effect.update(log_odds=candidate["baseline_log_odds"], change_log_odds=0)
    publish(org, abstention, 104)
    pair, = pairs(org)
    witness = pair["examples"][1]
    group, = get(base, "prefix-groups", org=org)["rows"]
    review = get(base, "prefix-check", org=org, design=group["design"],
                 **{key: witness[key] for key in ("trace", "call", "report")})["review"]
    assert not review["reversals"] and review["reference"]["edits"][witness["index"] - 1] == "lost"
    publish(org, left, 105, flip_target=True)
    assert not pairs(org)


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
        check_reference_pairs(base, scope)
        check_pair_controls(base, cases)
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
            assert not get(base, "prefix-reference-pairs", org=trace_org, design=g["design"])["rows"]
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
        check_reference_pairs(base, scope)
        assert get(base,"prefix-groups",**scope)["rows"] == [group]
    with sqlite3.connect(db) as conn:
        conn.execute("UPDATE prefix_checks SET choices=json_remove(choices,'$.reference_edits','$.expected','$.baseline_start')")
        conn.execute("PRAGMA user_version=29")
    with server(binary, db, "--capture-content") as base:
        check_reference_pairs(base, scope)
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
        assert len(get(base, "prefix-reference-pairs", design=g["design"])["rows"]) == 2
        for route in ["prefix-groups","prefix-references","prefix-cases","prefix-reference-pairs"]:
            params={"design":g["design"]}
            if route in ("prefix-references", "prefix-cases"):
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
