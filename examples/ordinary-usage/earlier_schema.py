"""Test a verified schema observed before the opening request. No tool executes."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import sqlite3

from catalog_edit import replay_arms


def variants(original, schema):
    arms = {"original": original, "repeat": original}
    neutral = ("This tool can inspect the attached database. "
               "Its description is available together with the other functions provided for this request. ")
    for label, note in (("candidate", schema), ("control", (neutral * (len(schema) // len(neutral) + 1))[:len(schema)])):
        body = deepcopy(original)
        tool, = [t["function"] for t in body["tools"] if t["function"]["name"] == "SQLite_schema"]
        tool["description"] += "\n" + note
        arms[label] = body
    return arms


def cases(directory):
    with sqlite3.connect(f"{(directory / 'history.db').resolve().as_uri()}?mode=ro", uri=True) as db:
        schema = "\n".join(row[0] + ";" for row in db.execute("SELECT sql FROM sqlite_schema WHERE type='table' ORDER BY rowid"))
    with sqlite3.connect(f"{(directory / 'sidecar.sqlite').resolve().as_uri()}?mode=ro", uri=True) as db:
        runs = [json.loads(raw) for (raw,) in db.execute("SELECT run FROM traces")]
    runs.sort(key=lambda r: min(c["started_at"] for c in r["calls"]))
    witnesses = {}
    for run in runs:
        calls = sorted(run["calls"], key=lambda c: c["started_at"])
        opening = json.loads(calls[0]["recorded_request"]["body"])
        catalog = json.dumps(opening["tools"], sort_keys=True)
        prior = witnesses.get(catalog)
        yield run["conversation_id"], calls[0], opening, prior, schema
        for call in calls:
            body = json.loads(call["recorded_request"]["body"])
            if body["tools"] != opening["tools"]:
                continue
            declarations = {}
            for index, message in enumerate(body["messages"]):
                if message["role"] == "assistant":
                    for tool in message.get("tool_calls", []):
                        declarations.setdefault(tool["id"], []).append(tool["function"]["name"])
                if (message["role"] == "tool" and message.get("content") == schema
                        and declarations.get(message.get("tool_call_id")) == ["SQLite_schema"]):
                    witnesses.setdefault(catalog, {"trace": run["id"], "call": call["id"], "pointer": f"/messages/{index}/content"})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--model-port", type=int, required=True)
    args = parser.parse_args()
    for session, call, opening, witness, schema in cases(args.directory):
        if not witness or json.dumps(schema)[1:-1] in json.dumps(opening):
            print(json.dumps({"session": session, "eligible": False}), flush=True)
            continue
        arms = variants(opening, schema)
        for seed in range(1, 16):
            result = replay_arms(call, f"http://127.0.0.1:{args.model_port}", int(session.split("-")[1]), seed, arms)
            print(json.dumps(dict(result, witness=witness)), flush=True)
