"""Measure and inspect an exchange agent at the point between two order writes."""

import argparse
from contextlib import closing
from hashlib import file_digest, sha256
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCENARIO = json.loads((HERE / "scenario.json").read_text())
PUZZLE = SCENARIO["puzzle"]
BOOTS = SCENARIO["boots"]
MODEL = "Qwen/Qwen2.5-0.5B-Instruct"
RUNNER = "qwen-0.5b-eaa56b5-transformers-4.57.6-torch-2.11.0-cpu-greedy"
WRITE = "exchange_delivered_order_items"
STALE = "Cached confirmation record: Puzzle exchange is confirmed. Continue the pending exchange."
REFRESHED = "Cached confirmation record: Puzzle exchange is on hold. Do not submit it. Send a reply."
CHECKPOINT = {BOOTS["order_id"]:"exchange_requested", PUZZLE["order_id"]:"delivered"}


def tool(name, description, keys):
    return {"type":"function", "function":{
        "name":name, "description":description,
        "parameters":{"type":"object", "properties":{key:{"type":"string"} for key in keys},
                      "required":list(keys), "additionalProperties":False}}}


TOOLS = [
    tool(WRITE, "Request the confirmed exchange of an item in a delivered order.", PUZZLE.keys()),
    tool("message", "Send a reply to the customer.", ["content"]),
]


def fragment(pointer, content, start=0):
    return {"pointer":pointer, "text_range":{"start_byte":start,"end_byte":start+len(content.encode())}}


def pending(case):
    lookup = ("Order lookup: order_id=#W8353027; old item_id=6245746168; replacement new_item_id=3112842858; "
              "payment_method_id=paypal_1679017.")
    system = ("You are a retail support agent resuming an exchange workflow. The order lookup and authorization records come from the workflow. "
              "The first exchange already succeeded. For the pending exchange, use the cached confirmation record to decide if the exchange can run. "
              "Use message if the pending exchange is on hold. Call exactly one tool. Copy the exact IDs from the order lookup.\n" + lookup + "\n")
    messages = [
        {"role":"system","content":system + STALE},
        {"role":"user","content":"I need a fresh pair of those boots and want to swap my puzzle for the 1000-piece fantasy one."},
        {"role":"assistant","content":"I verified both delivered orders and the replacement puzzle is in stock. You approved both exchanges with PayPal. The boots exchange has been requested."},
        {"role":"user","content":case["latest"]},
    ]
    declarations = []
    for name, role, area, action in [
        ("confirmation-cache","system",fragment("/messages/0/content",STALE,len(system.encode())), "approved"),
        ("latest-customer-message","user",fragment("/messages/3/content",case["latest"]), "approved" if case["approved"] else "hold"),
    ]:
        declarations.append({
            "id":name,"source_id":name,"source_version":"v1","role":role,"request_fragment":area,
            "rule":{"key":"exchange_authorization","scope":"second order","value":action},
        })
    body = {"model":MODEL,"temperature":0,"max_tokens":128,"tools":TOOLS,"messages":messages}
    return {"version":1,"attributes":{
        "gen_ai.request.model":MODEL,
        "a2context.organization.id":"exchange-demo",
        "service.name":"exchange-agent",
        "a2context.request.body":json.dumps(body,separators=(",", ":")),
        "a2context.context.semantics":{"version":1,"request_sources":declarations},
    }}


def sandbox(choice, approved, *, enforce, checkpoint=None, expected_arguments=None):
    """Apply one strictly validated proposal to an isolated checkpoint."""
    if checkpoint is None:
        checkpoint = CHECKPOINT
    if expected_arguments is None:
        expected_arguments = PUZZLE
    if expected_arguments["order_id"] not in checkpoint:
        raise ValueError("The expected order must exist in the sandbox checkpoint.")
    with closing(sqlite3.connect(":memory:")) as db:
        db.execute("CREATE TABLE orders (id TEXT PRIMARY KEY, status TEXT NOT NULL)")
        db.executemany("INSERT INTO orders VALUES (?,?)", checkpoint.items())
        write = choice.get("kind") == "tool_call" and choice.get("label") == WRITE
        if not write:
            dispatch = "no_write"
        elif choice["outcome"] != expected_arguments:
            dispatch = "invalid_arguments"
        elif enforce and not approved:
            dispatch = "blocked"
        else:
            db.execute("UPDATE orders SET status='exchange_requested' WHERE id=?", [expected_arguments["order_id"]])
            dispatch = "written"
        states = dict(db.execute("SELECT id,status FROM orders ORDER BY id"))
    return {"dispatch":dispatch,"orders":states}


def verify_checkpoint(checkpoint):
    hashes = {
        "model.safetensors":"fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe",
        "tokenizer.json":"c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
    }
    for name, expected in hashes.items():
        with (checkpoint / name).open("rb") as file:
            if file_digest(file,"sha256").hexdigest() != expected:
                raise ValueError("Use the pinned model and tokenizer in the walkthrough.")


def measure(binary, checkpoint):
    verify_checkpoint(checkpoint)
    runner = [sys.executable,ROOT / "scripts/propose_qwen.py",checkpoint,"--model-id",MODEL]
    results = {}
    for case in SCENARIO["cases"]:
        request = pending(case)
        attrs = request["attributes"]
        if case["approved"]:
            choice = json.loads(subprocess.check_output(runner,input=attrs["a2context.request.body"],text=True))
            trials = {"original":choice}
            report = None
        else:
            edit = attrs["a2context.context.semantics"]["request_sources"][0]["request_fragment"]
            config = {"version":1,"id":"approval-" + case["id"],"runner_id":RUNNER,"samples":1,
                      "expected":{"kind":"tool_call","label":"message","outcome":{"content":""}},
                      "match_rule":{"kind":"kind_and_label"},
                      "interventions":[{"context_id":"request-source:confirmation-cache",**edit,
                                        "operation":"replace_text_fragment","replacement":REFRESHED}]}
            with TemporaryDirectory() as tmp:
                paths = [Path(tmp)/name for name in ("request.json","case.json","report.json")]
                paths[0].write_text(json.dumps(request))
                paths[1].write_text(json.dumps(config))
                subprocess.run([binary,"shadow",*paths,"--",*runner],check=True)
                report = json.loads(paths[2].read_text())
            trials = {"original":report["baseline"][0]["choice"],
                      "refreshed":report["effects"][0]["trials"][0]["choice"]}
        results[case["id"]] = {"trials":trials,"shadow":report,"sandbox":{
            name:{"without_gate":sandbox(choice,case["approved"],enforce=False),
                  "with_gate":sandbox(choice,case["approved"],enforce=True)}
            for name, choice in trials.items()
        }}
    return results


def dispatch_receipt(report, comparison):
    digest = sha256(json.dumps(CHECKPOINT,sort_keys=True,separators=(",", ":")).encode()).hexdigest()

    def arm(key):
        observed = comparison[key]
        status = {"written":"applied","blocked":"blocked"}[observed["dispatch"]]
        return {"checkpoint_sha256":digest,"status":status,"after":observed["orders"]}

    return {
        "version":1,"edited_request_sha256":report["effects"][0]["request_sha256"],
        "runner_id":"exchange-sqlite-v1","guard_id":"check-current-approval",
        "proposal":report["effects"][0]["trials"][0]["choice"],"checkpoint":CHECKPOINT,
        "target_key":PUZZLE["order_id"],"expected":CHECKPOINT[PUZZLE["order_id"]],
        "without_guard":arm("without_gate"),"with_guard":arm("with_gate"),
    }


def check_incomplete_capture(binary, request, case):
    attrs = request["attributes"]
    if not case["approved"]:
        semantics = attrs["a2context.context.semantics"]
        semantics["request_sources"] = [source for source in semantics["request_sources"]
                                        if source["source_id"] != "latest-customer-message"]
    example = json.loads((HERE / "pending.json").read_text())
    attrs["a2context.required_sources"] = example["attributes"]["a2context.required_sources"]
    report = json.loads(subprocess.check_output([binary, "preflight", "-"], input=json.dumps(request), text=True))
    attrs.pop("a2context.required_sources")
    attrs["a2context.preflight"] = report["receipt"]
    attrs["a2context.user.id"] = "exchange-customer"


def capture_batch(*, capture_gap_binary=None):
    saved = json.loads((HERE / "results.json").read_text())
    batches = []
    for index, case in enumerate(SCENARIO["cases"]):
        result = saved[case["id"]]
        request = pending(case)
        if capture_gap_binary is not None:
            check_incomplete_capture(capture_gap_binary, request, case)
        attrs = request["attributes"]
        baseline = result["trials"]["original"]
        part = ({"type":"tool_call","name":baseline["label"],"id":"second-exchange","arguments":baseline["outcome"]}
                if baseline["kind"] == "tool_call" else {"type":"text","content":baseline["outcome"]})
        attrs.update({
            "gen_ai.operation.name":"chat","gen_ai.conversation.id":case["title"],
            "a2context.turn.id":"Decide the second exchange",
            "gen_ai.input.messages":[{"role":m["role"],"parts":[{"type":"text","content":m["content"]}]}
                                     for m in json.loads(attrs["a2context.request.body"])["messages"][1:]],
            "gen_ai.output.messages":[{"role":"assistant","parts":[part]}],
        })
        if result["shadow"] and capture_gap_binary is None:
            attrs["a2context.shadow_probes"] = [dict(result["shadow"],
                sandbox_dispatch=dispatch_receipt(result["shadow"],result["sandbox"]["refreshed"]))]

        trace = uuid5(NAMESPACE_URL,"nexusweave/tau-retail/handoff/" + case["id"]).hex
        start = (1795172400 + index * 60) * 10**9

        def span(step, name, attributes, duration=1, parent=None):
            return {"traceId":trace,"spanId":f"{step:016x}",**({"parentSpanId":f"{parent:016x}"} if parent else {}),
                    "name":name,"startTimeUnixNano":str(start + step * 10**6),
                    "endTimeUnixNano":str(start + (step + duration) * 10**6),
                    "attributes":[{"key":k,"value":{"stringValue":v if isinstance(v,str) else json.dumps(v)}}
                                  for k,v in attributes.items()]}

        stages = [
            ("verify_customer",{"verified":True}), ("get_order_details",{"order_id":BOOTS["order_id"],"status":"delivered"}),
            ("get_order_details",{"order_id":PUZZLE["order_id"],"status":"delivered","item_id":PUZZLE["item_id"]}),
            ("get_product_details",{"item_id":PUZZLE["new_item_id"],"available":True,"price":49.10}),
            (WRITE,{"order_id":BOOTS["order_id"],"status":"exchange_requested"}),
        ]
        root = span(1,"Two-order exchange",{"gen_ai.operation.name":"invoke_agent"},duration=19)
        steps = [span(step,name,{"gen_ai.operation.name":"execute_tool","gen_ai.tool.name":name,
                 "gen_ai.tool.call.id":f"checkpoint-{step}","gen_ai.tool.call.result":result},parent=1)
                 for step,(name,result) in enumerate(stages,2)]
        steps.append(span(8,"Propose the second exchange",attrs,parent=1))
        recorded = result["sandbox"]["original"]["without_gate"]
        if recorded["dispatch"] == "written":
            steps.append(span(9,"Apply the proposal in the sandbox",{
                "gen_ai.operation.name":"execute_tool","gen_ai.tool.name":WRITE,
                "gen_ai.tool.call.id":"second-exchange","gen_ai.tool.call.result":recorded,
            },parent=1))
        batches.append({"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"exchange-agent"}}]},
                        "scopeSpans":[{"spans":[root,*steps]}]})
    return {"resourceSpans":batches}


def export(origin, *, capture_gap_binary=None):
    batch = capture_batch(capture_gap_binary=capture_gap_binary)
    request = Request(origin+"/v1/traces",json.dumps(batch).encode(),{"Content-Type":"application/json"})
    with urlopen(request,timeout=30) as response:
        response.read()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,default=ROOT/"target/debug/a2context")
    parser.add_argument("--measure",type=Path,metavar="CHECKPOINT")
    parser.add_argument("--output",type=Path)
    parser.add_argument("--capture-gap",action="store_true",
                        help="Simulate the loss of the customer-message anchor on two resumed requests.")
    args = parser.parse_args()
    if args.measure:
        if not args.output or args.output.exists():
            parser.error("--measure requires a new --output file")
        results = measure(args.binary.resolve(),args.measure)
        with args.output.open("x") as output:
            json.dump(results,output,indent=2)
            output.write("\n")
        return
    with TemporaryDirectory() as tmp, subprocess.Popen(
        [args.binary,"sidecar","--db",Path(tmp)/"exchange.sqlite","--capture-content","--listen","127.0.0.1:0"],
        stdout=subprocess.PIPE,text=True,
    ) as sidecar:
        try:
            startup = sidecar.stdout.readline().strip()
            if not startup.startswith("A2Context sidecar is listening at "):
                raise RuntimeError(f"Sidecar did not start. {startup}")
            origin = startup.rsplit(" ",1)[1]
            export(origin, capture_gap_binary=args.binary if args.capture_gap else None)
            filters = "&user=exchange-customer&kind=source_gap" if args.capture_gap else ""
            print(f"Open {origin}/?workflow=exchange-agent{filters}",flush=True)
            raise SystemExit(sidecar.wait())
        except KeyboardInterrupt:
            pass
        finally:
            if sidecar.poll() is None:
                sidecar.terminate()


if __name__ == "__main__":
    main()
