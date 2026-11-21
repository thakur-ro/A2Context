"""Compare explicit workflow scope at tool presentation and at native dispatch."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys
import subprocess

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from runner import inputs, load_model, reference_replies, trial
from inputs import BASE, calls, digest, new_sim, select
from run import save
from replay import logger

GROUPS = {"held_out": [167, 182, 192, 194], "development": [150], "booking_control": [155, 163, 179]}
QUOTE = {"list_all_airports", "get_nearest_airport_by_city", "get_flight_cost"}


def load_cases(root):
    # The shared loader verifies all upstream hashes before importing the simulator.
    _, _, schemas, cls = select(root)
    data = root / BASE / "data"
    def rows(name):
        return {row["id"]: row for row in map(json.loads, (data / name).read_text().splitlines())}
    tasks = rows("BFCL_v4_multi_turn_base.json")
    refs = rows("possible_answer/BFCL_v4_multi_turn_base.json")
    names = {s["name"] for s in schemas}
    for group, numbers in GROUPS.items():
        for number in numbers:
            tid = f"multi_turn_base_{number}"
            task = tasks[tid]
            assert task["involved_classes"] == ["TravelAPI"] and not task.get("excluded_function")
            ref = calls(refs[tid]["ground_truth"][0], names)
            state = task["initial_config"]["TravelAPI"]
            sim = new_sim(cls, state)
            outputs = [getattr(sim, name)(**args) for name, args in ref]
            yield group, {"id":tid, "messages":task["question"][0], "state":state,
                          "ref":ref, "outputs":outputs}, schemas, cls


def run_arm(model, tokenizer, pkt, schemas, cls, allowed, arm, scripted=None):
    visible = [s for s in schemas if s["name"] in allowed] if arm == "scoped" else schemas
    audit = []
    row = trial(model, tokenizer, pkt, visible, cls, audit=audit, scripted=scripted,
                dispatch_allowed=None if arm == "full" else allowed)
    callbacks = [cb for entry in audit for cb in entry["callbacks"]]
    for event, cb in zip(row["calls"], callbacks, strict=True):
        event.update(in_scope=event["name"] in allowed, before=cb["before"], after=cb["after"],
                     feedback_sha256=digest(cb["returned"]))
    for receipt in row["generations"]:
        del receipt["decode_seconds"]
    return row


def catalog_check(binary, pkt, visible, allowed):
    messages, tools = inputs(pkt, visible)
    body = json.dumps({"messages": messages, "tools": tools}, ensure_ascii=False)
    envelope = {"request": {"version": 1, "attributes": {"a2context.request.body": body}},
                "allowed_tools": sorted(allowed)}
    output = subprocess.run([binary, "inspect-pending", "-"], input=json.dumps(envelope),
                            capture_output=True, text=True, check=True)
    scope = json.loads(output.stdout)["tool_scope"]
    expected = [{"name": tool["name"], "index": i} for i, tool in enumerate(tools) if tool["name"] not in allowed]
    if scope != {"captured": len(tools), "outside": expected, "unclassified": []}:
        raise ValueError("Rust catalog check differs from the workflow's registered functions.")
    return {"catalog_sha256": digest(tools), **scope}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("upstream", "snapshot", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--repeat", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/a2context"))
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use an unused output path.")
    model, tokenizer = load_model(args.snapshot)
    report = {"protocol_sha256":sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
              "model_files_sha256":sha256((HERE.parent / "files.json").read_bytes()).hexdigest(), "cases":[]}
    for group, pkt, schemas, cls in load_cases(args.upstream):
        booking = group == "booking_control"
        allowed = QUOTE | {"book_flight"} if booking else QUOTE
        ref = reference_replies(tokenizer, pkt)
        positive = run_arm(None, tokenizer, pkt, schemas, cls, allowed, "gate", ref)
        if not positive["usable"]:
            raise ValueError("Reference control failed on " + pkt["id"])
        row = {"id":pkt["id"], "group":group, "reference":positive}
        if booking:
            blocked = run_arm(None, tokenizer, pkt, schemas, cls, QUOTE, "gate", ref)
            assert any(e["status"] == "outside_workflow_scope" and e["name"] == "book_flight" for e in blocked["calls"])
            row["quote_on_booking"] = blocked
        row["catalogs"] = {"full":catalog_check(args.binary, pkt, schemas, allowed),
                          "scoped":catalog_check(args.binary, pkt, [s for s in schemas if s["name"] in allowed], allowed)}
        row["arms"] = {arm:run_arm(model, tokenizer, pkt, schemas, cls, allowed, arm)
                       for arm in ("full", "gate", "scoped")}
        assert row["arms"]["full"]["generations"][0] == row["arms"]["gate"]["generations"][0]
        report["cases"].append(row)
        print(pkt["id"], "complete", flush=True)
    if args.repeat:
        if report != json.loads(args.repeat.read_text()):
            raise ValueError("Fresh run differs from the recorded experiment.")
        report = {"report_sha256":sha256(args.repeat.read_bytes()).hexdigest(),
                  "matched":[r["id"] for r in report["cases"]]}
    save(args.output, report)


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
