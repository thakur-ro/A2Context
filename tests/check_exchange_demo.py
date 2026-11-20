"""Check the recorded model comparisons and order-state transitions in the demo."""

import importlib.util
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/exchange-approval"
sys.path.insert(0,str(EXAMPLE.parents[1] / "scripts"))
from check_sidecar import get, send_traces, server
spec = importlib.util.spec_from_file_location("exchange_demo", EXAMPLE / "demo.py")
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

saved = json.loads((EXAMPLE / "results.json").read_text())
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
with TemporaryDirectory() as tmp, server(Path(sys.argv[1]).resolve(),Path(tmp)/"exchange.sqlite","--capture-content") as base:
    demo.export(base)
    scope = {"workflow":"exchange-agent"}
    impact = get(base,"impact",**scope)
    assert (impact["conversations"],impact["captured_conversations"],impact["evidence_counts"]["tested"]) == (2,3,2)
    assert impact["sandbox_dispatch_conversations"] == 2
    factor = get(base,"factors",**scope)["rows"][0]
    assert factor["signal"] == "sandbox_dispatch" and factor["sandbox_conversations"] == 2
    cards = {card["kind"]:card for card in get(base,"findings",**scope)["rows"]}
    assert cards["conflict"]["threads"] == cards["shadow"]["threads"] == 2
    evidence = cards["shadow"]["evidence"]
    pair = evidence["detail"]["response_pair"]
    assert pair["changed"] is False and pair["original"]["label"] == demo.WRITE
    dispatch = evidence["detail"]["sandbox_dispatch"]
    assert dispatch["protected"] and dispatch["target"] == demo.PUZZLE["order_id"]
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
