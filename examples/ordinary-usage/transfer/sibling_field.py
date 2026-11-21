"""Use the sidecar evidence to test a neighboring tool's property name."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import sqlite3

from check_proposals import send
from proposal_trials import trials


def summarize(message, target):
    calls = message.get("tool_calls") or []
    proposals = []
    for call in calls:
        fn = call.get("function", {})
        args = fn.get("arguments")
        if isinstance(args,str):
            try:
                args = json.loads(args)
            except ValueError:
                args = None
        proposals.append({"tool":fn.get("name"), "fields":sorted(args) if isinstance(args,dict) else None})
    fields = {k for p in proposals if p["tool"] == target for k in (p["fields"] or [])}
    return {"proposals":proposals, "old":"table_names" in fields, "alias":"entry_names" in fields}


def run(folder, sidecar, model):
    with sqlite3.connect((folder/"sidecar.sqlite").resolve().as_uri()+"?mode=ro",uri=True) as db:
        for name, raw in db.execute("SELECT thread,run FROM traces ORDER BY thread"):
            call = max(json.loads(raw)["calls"],key=lambda c:c["started_at"])
            decision, = call["decisions"]
            original = json.loads(call["recorded_request"]["body"])
            target = decision["label"]
            if decision["kind"] != "tool_call":
                continue
            check = send(sidecar,"/api/sidecar/check-proposal",
                         {"request":original,"tool":target,"arguments":decision["outcome"]})[1]
            matches = check.get("other_declarations", [])
            if not matches:
                yield {"session":name,"excluded":"No supplied field declared by another tool."}
                continue
            match, = matches
            old, new = match["field"], "entry_names"
            assert old == "table_names" and len(new) == len(old)
            assert not any(t["function"]["name"] == match["tool"] for m in original["messages"] for t in m.get("tool_calls",[]))
            edit = deepcopy(original)
            fn, = [t["function"] for t in edit["tools"] if t["function"]["name"] == match["tool"]]
            schema = fn["parameters"]
            schema["properties"] = {new if k == old else k:v for k,v in schema["properties"].items()}
            schema["required"] = [new if k == old else k for k in schema["required"]]
            if new in json.dumps(original) or old in json.dumps(edit):
                yield {"session":name,"excluded":"The renamed field also occurs outside the source."}
                continue
            arms = {"original":original,"repeat":original,"edit":edit}
            yield from trials(arms,name,call["id"],model,lambda m: summarize(m,target))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder",type=Path)
    parser.add_argument("--model-port",type=int,required=True)
    parser.add_argument("--sidecar-port",type=int,required=True)
    args = parser.parse_args()
    for row in run(args.folder,f"http://127.0.0.1:{args.sidecar_port}",f"http://127.0.0.1:{args.model_port}"):
        print(json.dumps(row),flush=True)
