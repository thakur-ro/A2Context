"""Verify recurring target misses through OTLP, SQLite recovery and scoped report lookup."""
import json
from pathlib import Path
from urllib.parse import urlencode
import sqlite3
import sys
import tempfile
from time import perf_counter

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from check_sidecar import attrs, case_span, export, get, sdk_sender, send, send_traces, server


def experiment(name, fixture, thread=None, modify=None, scores=False):
    folder = ROOT / "examples/context-screening/replay"
    path = folder / ("fresh-target-tokens.json" if fixture == "fresh" else fixture + "-target.json")
    envelope = json.loads(path.read_text())
    report = envelope["shadow"]
    fields = envelope["request"]["attributes"]
    if scores:
        candidate = envelope.get("candidate") or json.loads((folder / "target-token-scores.json").read_text())[fixture]
        if scores == "singles":
            del candidate["joint_request_sha256"]
            assert candidate["scores"]["conditions"].pop()["id"] == "joint"
        fields["nexusweave.candidate_probes"] = [candidate]
    if modify:
        modify(report)
    body = json.loads(fields["nexusweave.request.body"])
    fields.update({"gen_ai.operation.name": "chat", "nexusweave.shadow_probes": [report],
                   "nexusweave.user.id": "support-17", "nexusweave.organization.id": "journal",
                   "gen_ai.input.messages": [{"role": m["role"], "parts": [{"type": "text", "content": m["content"]}]}
                                             for m in body["messages"]]})
    if thread:
        fields["gen_ai.conversation.id"] = thread
    span = case_span(name, "undeclared", org="journal")
    span["attributes"] = attrs({k: v if isinstance(v, str) else json.dumps(v) for k,v in fields.items()})
    return span


def partial(report):
    report["pair"].update(trials=[{"status":"failed"}], summary={"attempted":1,"completed":0,"matched":0},
                          difference_pp=None, interaction_pp=None)


def check(binary, db, sdk=None):
    def with_stale_digest(report):
        report["captured_request_sha256"] = "9" * 64

    first = experiment("first", "earlier", "first")
    followup = experiment("followup", "followup", "followup", scores="singles")
    unopened = experiment("unopened", "unopened", "unopened", scores=True)
    missing = experiment("missing", "unopened", "missing", partial)
    mixed = [experiment("mixed-a", "earlier", "mixed"), experiment("mixed-b", "unopened", "mixed")]
    pending = [experiment("unfinished-a", "followup", "unfinished"), experiment("unfinished-b", "unopened", "unfinished", partial)]
    stale = experiment("stale", "unopened", "stale", with_stale_digest)
    anon = experiment("anonymous", "unopened")
    other_runner = experiment("runner", "unopened", "runner", lambda r: r.update(runner_id="other-runner"))
    with server(binary, db, "--capture-content") as base:
        send_traces(base, export(unopened, first, unopened))
        assert not get(base, "target-patterns")["rows"]
        (sdk_sender(sdk) if sdk else send_traces)(base,export(followup))
        send_traces(base, export(missing, *mixed, *pending, stale, anon, other_runner))
        group, = get(base, "target-patterns", org="journal", user="support-17")["rows"]
        assert [group[k] for k in ("missed","missed_inputs","reached","varied","incomplete","conversations")] == [2,2,1,1,2,6], group
        assert group["setup"]["expected"]["outcome_preview"] == "45"
        assert group["miss_signals"] == {"opening":1,"suffix":0,"other":0,"unscored":1,"varied":0}
        design = group["design"]
        query = {"design":design, "user":"support-17"}
        examples = get(base,"target-examples",**query)["rows"]
        assert len(examples) == 8 and [r["result"] for r in examples[:2]] == ["missed","missed"]
        assert [r["thread"] for r in get(base,"target-examples",**query,miss_signal="opening")["rows"]] == ["unopened"]
        row = next(r for r in examples if r["thread"] == "unopened")
        assert "unopened" in row["question"]["text"]
        location = {k:row[k] for k in ("trace","call","report")}
        result = get(base,"target-check",**query,**location)
        assert result["trials"]["pair"]["trials"][0]["choice"]["outcome"] == "120"
        assert result["check"]["joint"]["status"] == "missed" and "unopened" in result["question"]["text"]
        assert not get(base,"target-patterns",before="2026-10-12")["rows"]
        assert not get(base,"target-examples",design=design,org="unrelated")["rows"]
        for wrong in ({"org":"elsewhere"},{"design":"unknown"},{"report":"missing"},{"user":"someone-else"},{"miss_signal":"suffix"}):
            send(base,"/api/sidecar/target-check?"+urlencode({**query,**location,**wrong}),expected=404)
        # A second report on the identical request cannot increase the distinct input count.
        copy = experiment("copy", "followup", "copy", scores=True)
        send_traces(base, export(copy))
        current, = get(base,"target-patterns")["rows"]
        assert (current["missed"],current["missed_inputs"]) == (3,2)
        send_traces(base, export(experiment("fresh", "fresh", "fresh"),
                                 experiment("also-fresh", "fresh", "copy")))
        current, = get(base,"target-patterns")["rows"]
        assert current["miss_signals"] == {"opening":1,"suffix":1,"other":0,"unscored":1,"varied":1}, current
        assert [r["thread"] for r in get(base,"target-examples",**query,miss_signal="varied")["rows"]] == ["copy","copy"]
    for version in (18, 19):
        with sqlite3.connect(db) as conn:
            conn.execute("DELETE FROM target_experiments")
            conn.execute(f"PRAGMA user_version={version}")
        with server(binary, db, "--capture-content") as base:
            assert get(base,"target-patterns")["rows"] == [current]
    with sqlite3.connect(db) as conn:
        conn.execute("UPDATE traces SET seen=1 WHERE id=?", (location["trace"],))
    with server(binary, db, "--capture-content") as base:
        latest, = get(base,"target-patterns")["rows"]
        assert (latest["missed"],latest["missed_inputs"]) == (3,2)
        # A test double changes the sampled completion only. The saved joint
        # score still puts "4" first, so this case must not enter the opening group.
        def different_completion(report):
            report["pair"]["trials"][0]["choice"]["outcome"] = "63"
        send_traces(base,export(experiment("other", "fresh", "other", different_completion, scores=True)))
        other, = get(base,"target-examples",design=design,miss_signal="other")["rows"]
        assert other["thread"] == "other"
    with server(binary, db.parent / "redacted.sqlite") as base:
        send_traces(base,export(first,followup,unopened))
        assert not get(base,"target-patterns")["rows"]
        assert not get(base,"target-examples",design=design)["rows"]


def cost(binary, db):
    from statistics import median
    spans = [experiment(str(i),"earlier" if i % 4 == 0 else "unopened",str(i),scores=True) for i in range(128)]
    with server(binary,db,"--capture-content") as base:
        for i in range(0,len(spans),8):
            send_traces(base, export(*spans[i:i+8]))
        times = []
        for i in range(21):
            start = perf_counter()
            group, = get(base,"target-patterns")["rows"]
            page = get(base,"target-examples",design=group["design"],miss_signal="opening")
            assert group["miss_signals"]["opening"] == 96 and group["missed_inputs"] == 1 and page["next_offset"] == 15
            if i:
                times.append((perf_counter() - start)*1000)
        return {"conversations":len(spans),"missed":group["missed"],"distinct_missed_inputs":group["missed_inputs"],
                "median_ms":round(median(times),3),"p95_ms":round(sorted(times)[18],3)}


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sdk-python")
    parser.add_argument("--cost", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        db = Path(directory) / "study.sqlite"
        if args.cost:
            print(json.dumps(cost(args.binary, db)))
        else:
            check(args.binary, db, args.sdk_python)
            print("Verified recurring target misses, incomplete and varying experiments, scoped trial access, retries, migration, retention and redaction.")
