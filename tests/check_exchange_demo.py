"""Check the recorded model comparisons and order-state transitions in the demo."""

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/exchange-approval"
sys.path.insert(0,str(EXAMPLE.parents[1] / "scripts"))
from check_sidecar import get, send, send_traces, server
spec = importlib.util.spec_from_file_location("exchange_demo", EXAMPLE / "demo.py")
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

saved = json.loads((EXAMPLE / "results.json").read_text())
pending = json.loads((EXAMPLE / "pending.json").read_text())
pending_measurement = json.loads((EXAMPLE / "pending-measurement.json").read_text())
for case in demo.SCENARIO["cases"]:
    result = saved[case["id"]]
    for arm, choice in result["trials"].items():
        assert result["sandbox"][arm] == {
            "without_gate":demo.sandbox(choice,case["approved"],enforce=False),
            "with_gate":demo.sandbox(choice,case["approved"],enforce=True),
        }
        if not case["approved"]:
            assert choice == {"kind":"tool_call","label":demo.WRITE,"outcome":demo.PUZZLE}
            measured = result["sandbox"][arm]
            assert measured["without_gate"]["dispatch"] == "written"
            assert measured["with_gate"] == {"dispatch":"blocked","orders":{
                demo.BOOTS["order_id"]:"exchange_requested",demo.PUZZLE["order_id"]:"delivered"}}
    if not case["approved"]:
        report = result["shadow"]
        assert report["baseline"][0]["choice"] == result["trials"]["original"]
        assert report["effects"][0]["trials"][0]["choice"] == result["trials"]["refreshed"]

# The exact write found in the public trace still passes when approved. A write
# with a different order or item never hits the sandbox mutation.
valid = {"kind":"tool_call","label":demo.WRITE,"outcome":demo.PUZZLE}
assert demo.sandbox(valid,True,enforce=True)["dispatch"] == "written"
assert demo.sandbox(valid,False,enforce=True)["dispatch"] == "blocked"
wrong = dict(valid,outcome=dict(demo.PUZZLE,new_item_id="wrong-item"))
assert demo.sandbox(wrong,True,enforce=True)["dispatch"] == "invalid_arguments"
controls = pending_measurement["dispatch_controls"]
for key, approved in [("still_on_hold",False),("approved_after_request",True)]:
    row = controls[key]
    assert row == {"unchecked":demo.sandbox(valid,approved,enforce=False),
                   "request_snapshot":demo.sandbox(valid,False,enforce=True),
                   "current_approval":demo.sandbox(valid,approved,enforce=True)}
    assert row["request_snapshot"]["orders"][demo.PUZZLE["order_id"]] == "delivered"
    assert row["current_approval"]["orders"][demo.PUZZLE["order_id"]] == ("exchange_requested" if approved else "delivered")
    for result in row.values():
        assert result["orders"][demo.BOOTS["order_id"]] == "exchange_requested"
with TemporaryDirectory() as tmp, server(Path(sys.argv[1]).resolve(),Path(tmp)/"exchange.sqlite","--capture-content") as base:
    demo.export(base)
    scope = {"workflow":"exchange-agent"}
    impact = get(base,"impact",**scope)
    assert (impact["conversations"],impact["captured_conversations"],impact["evidence_counts"]["tested"]) == (2,3,2)
    assert impact["recommendations"] == 2
    assert get(base,"impact",workflow="unrelated")["recommendations"] == 0
    assert impact["sandbox_dispatch_conversations"] == 2
    assert impact["sandbox_tested_conversations"] == 2
    factor = get(base,"factors",**scope)["rows"][0]
    assert factor["signal"] == "sandbox_dispatch" and factor["sandbox_conversations"] == 2
    cards = {card["kind"]:card for card in get(base,"findings",**scope)["rows"]}
    assert cards["conflict"]["threads"] == cards["shadow"]["threads"] == 2
    by_source = get(base,"findings",terms="latest-customer-message",**scope)["rows"]
    assert [(row["id"],row["threads"]) for row in by_source] == [(cards["conflict"]["id"],2)]
    assert get(base,"findings",terms="latest-customer-message",workflow="unrelated")["rows"] == []
    evidence = cards["shadow"]["evidence"]
    pair = evidence["detail"]["response_pair"]
    assert pair["changed"] is False and pair["original"]["label"] == demo.WRITE
    dispatch = evidence["detail"]["sandbox_dispatch"]
    assert dispatch["protected"] and dispatch["target"] == demo.PUZZLE["order_id"]
    design, = get(base,"dispatch-designs",**scope)["rows"]
    assert design["design"] == dispatch["design"] and design["tested"] == 2
    assert design["outcomes"]["matched_with_check"] == 2
    assert design["setup"]["tool"] == demo.WRITE
    assert get(base,"dispatch-designs",workflow="unrelated")["rows"] == []
    assert (dispatch["without_guard"],dispatch["with_guard"]) == ("applied","blocked")
    assert [row for row in dispatch["states"] if row["before"] != row["without_guard"]] == [{
        "key":demo.PUZZLE["order_id"],"before":"delivered","without_guard":"exchange_requested","with_guard":"delivered",
    }]
    record = get(base,"trace",id=evidence["trace_id"])
    calls = record["trace"]["calls"]
    decision, = next(call for call in calls if call["id"] == evidence["call_id"])["decisions"]
    assert decision["outcome"] == demo.PUZZLE
    linked, = (result for call in calls for result in call["tool_results"]
               if result["tool_call_id"] == decision["tool_call_id"])
    assert json.loads(linked["outcome"])["orders"][demo.PUZZLE["order_id"]] == "exchange_requested"

    def preview(request):
        result = send(base,"/api/sidecar/preview",{"request":request})[1]
        return result, result["history"].get("dispatch",{}).get("rows",[])

    current, history = preview(pending)
    assert [r["kind"] for r in current["recommendations"]] == ["conflict"]
    earlier, = history
    assert earlier["inputs"] == earlier["helped"] == 2
    witness = earlier["example"]
    match, = get(base,"findings",**{k:witness[k] for k in ("group","factor","trace","call","report")})["rows"]
    assert match["evidence"]["detail"]["sandbox_dispatch"]["protected"]

    fresh = get(base,"preview-example",scenario="exchange-measured")
    assert fresh["unverified_probes"] == [] and fresh["history"]["dispatch"]["rows"] == history
    review, = fresh["prefix_reviews"]
    assert review["reversals"] == [] and all(row["preferred"] == 0 for row in [*review["conditions"],review["joint"]])
    assert review["block_probe"]["status"] == "matched"
    probe = review["block_probe"]["probe"]
    assert probe["blocks"] == 24 and len(probe["effects"]) == 2
    for effect in probe["effects"]:
        assert all(value > 0 for sweep in effect["swaps"].values() for value in sweep)
    run, = review["runs"]
    proposals = [row["tool_proposal"] for row in [*run["rows"],run["joint"]]]
    assert all(proposal["name"] == demo.WRITE for proposal in proposals)
    assert proposals[0] == proposals[1] and proposals[2] == proposals[3]
    assert json.loads(proposals[1]["arguments"]) == demo.PUZZLE
    assert json.loads(proposals[2]["arguments"])["new_item_id"] == demo.PUZZLE["order_id"]
    dispatch, = [item["detail"]["sandbox_dispatch"] for item in fresh["recommendations"] if "sandbox_dispatch" in item["detail"]]
    assert dispatch["outcome"] == "matched_with_check" and dispatch["protected"]
    assert get(base,"impact",**scope) == impact

    # Historical evidence must match both ownership and the declared inputs.
    for key, value in [("service.name","another-workflow"),("nexusweave.organization.id","another-org"),
                       ("nexusweave.user.id","another-user"),("nexusweave.history.source","langsmith")]:
        changed = deepcopy(pending)
        changed["attributes"][key] = value
        assert preview(changed)[1] == []
    wrong_scope = dict(pending,scope={"key":"exchange_authorization","scope":"first order"})
    assert preview(wrong_scope)[1] == []
    changed = deepcopy(pending)
    declarations = changed["attributes"]["nexusweave.context.semantics"]["request_sources"]
    declarations[1]["rule"]["value"] = "approved"
    assert preview(changed)[1] == []
    declarations[1]["rule"]["value"] = "hold"
    declarations[1]["source_version"] = "v2"
    assert preview(changed)[1] == []
    existing = demo.SCENARIO["cases"][0]
    assert preview(demo.pending(existing))[1] == []

def different_proposal(receipt):
    receipt["proposal"]["outcome"]["order_id"] = "#other-order"

def different_checkpoint(receipt):
    receipt["without_guard"]["checkpoint_sha256"] = "0" * 64

def blocked_but_changed(receipt):
    receipt["with_guard"]["after"][demo.PUZZLE["order_id"]] = "exchange_requested"

with TemporaryDirectory() as tmp, server(Path(sys.argv[1]).resolve(),Path(tmp)/"invalid.sqlite","--capture-content") as base:
    for case_index, corrupt in enumerate((different_proposal,different_checkpoint,blocked_but_changed),1):
        batch = demo.capture_batch()
        spans = batch["resourceSpans"][0]["scopeSpans"][0]["spans"]
        field, = (value for span in spans for value in span["attributes"] if value["key"] == "nexusweave.shadow_probes")
        reports = json.loads(field["value"]["stringValue"])
        corrupt(reports[0]["sandbox_dispatch"])
        field["value"]["stringValue"] = json.dumps(reports)
        for span in spans:
            span["traceId"] = f"{case_index:032x}"
        send_traces(base,{"resourceSpans":batch["resourceSpans"][:1]})
    rejected = get(base,"findings",kind="shadow")["rows"]
    assert rejected[0]["occurrences"] == 3 and "sandbox_dispatch" not in rejected[0]["evidence"]["detail"]
    assert get(base,"impact")["sandbox_dispatch_conversations"] == 0
print("Verified saved exchanges, revoked and approved dispatch, and both order states.")


def capture_variant(index, thread, configure):
    """Use the verified proposal as a fixture for alternate sandbox conditions."""
    resource = demo.capture_batch()["resourceSpans"][0]
    spans = resource["scopeSpans"][0]["spans"]
    attributes = {a["key"]:a["value"] for span in spans for a in span["attributes"]}
    field = attributes["nexusweave.shadow_probes"]
    report, = json.loads(field["stringValue"])
    configure(report)
    field["stringValue"] = json.dumps([report])
    attributes["gen_ai.conversation.id"]["stringValue"] = thread
    for span in spans:
        span["traceId"] = f"{index:032x}"
    return {"resourceSpans":[resource]}


def approved_write(report, allow_with_check):
    receipt = report["sandbox_dispatch"]
    receipt["expected"] = "exchange_requested"
    if allow_with_check:
        receipt["with_guard"] = receipt["without_guard"]


def stale_approval(report):
    receipt = report["sandbox_dispatch"]
    receipt["with_guard"] = receipt["without_guard"]


def other_guard(report):
    report["sandbox_dispatch"]["guard_id"] = "another-dispatch-check"


def summary_for(base, filters):
    return {row["outcome"]:row["conversations"]
            for row in get(base,"dispatch-summary",**filters)["rows"]}


with TemporaryDirectory() as tmp:
    database = Path(tmp)/"dispatch.sqlite"
    binary = Path(sys.argv[1]).resolve()
    with server(binary,database,"--capture-content") as base:
        demo.export(base)
        group = get(base,"findings",kind="shadow")["rows"][0]
        detail = group["evidence"]["detail"]
        settings = dict(group=group["id"],factor=detail["factor"],design=detail["sandbox_dispatch"]["design"])
        assert summary_for(base,settings) == {"matched_with_check":2}

        for i, (thread, configure) in enumerate([
            ("Approved and applied",lambda report: approved_write(report,True)),
            ("Approved but blocked",lambda report: approved_write(report,False)),
            ("Stale approval at dispatch",stale_approval),
            ("Hold the puzzle",stale_approval),
            ("Wait on the second exchange",lambda report: None),
            ("Different check",other_guard),
            ("Proposal only",lambda report: report.pop("sandbox_dispatch")),
        ],101):
            send_traces(base,capture_variant(i,thread,configure))
        expected = {"matched_with_check":1,"matched_without_check":1,"matched_both":1,
                    "missed_both":1,"mixed":1,"unmeasured":2}
        assert summary_for(base,settings) == expected
        assert summary_for(base,dict(settings,org="unrelated")) == {}
        designs = get(base,"dispatch-designs")["rows"]
        assert len(designs) == 2
        first, second = designs
        assert first["design"] == settings["design"] and first["tested"] == 5
        assert first["outcomes"] == {name:count for name,count in expected.items() if name != "unmeasured"}
        assert second["setup"]["guard"] == "another-dispatch-check" and second["tested"] == 1
        assert get(base,"impact")["sandbox_tested_conversations"] == 6
        assert get(base,"dispatch-designs",org="unrelated")["rows"] == []
        row, = get(base,"dispatch-conversations",**settings,outcome="mixed")["rows"]
        assert row["thread"] == "Hold the puzzle"
        assert {example["outcome"] for example in row["examples"]} == {"matched_with_check","missed_both"}
        for example in row["examples"]:
            witness, = get(base,"findings",group=group["id"],factor=settings["factor"],
                           trace=example["trace"],call=example["call"],report=example["report"])["rows"]
            assert witness["evidence"]["detail"]["sandbox_dispatch"]["outcome"] == example["outcome"]
        assert get(base,"impact")["sandbox_regression_conversations"] == 1
        assert get(base,"factors")["rows"][0]["signal"] == "sandbox_regression"
        history = send(base,"/api/sidecar/preview",{"request":pending})[1]["history"]
        earlier, = history["dispatch"]["rows"]
        assert earlier["inputs"] == 2 and earlier["mixed"] == earlier["helped"] == 1

    with sqlite3.connect(database) as db:
        db.execute("UPDATE observations SET detail=json_remove(detail,'$.dispatch_history')")
        db.execute("PRAGMA user_version=32")
    with server(binary,database,"--capture-content") as base:
        assert summary_for(base,settings) == expected
        assert get(base,"dispatch-designs")["rows"] == designs
        assert send(base,"/api/sidecar/preview",{"request":pending})[1]["history"] == history

print("Verified scoped dispatch outcomes, mixed witnesses, and database upgrade.")
