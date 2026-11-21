"""Replay the registered tool-description edit at a loopback MLX endpoint."""

import argparse
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import random
import sqlite3
import sys
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_sidecar import send


def replay_arms(call, endpoint, number, seed, variants):
    names = {item["function"]["name"] for item in variants["original"]["tools"]}
    arms = list(variants)
    random.Random(f"{number}/{seed}").shuffle(arms)
    row = {"session": f"tools-{number}", "call": call["id"], "seed": seed, "order": arms}
    for arm in arms:
        try:
            request = dict(variants[arm], temperature=0.5, seed=seed, max_tokens=512)
            status, reply = send(endpoint, "/v1/chat/completions", request, expected=None)
            if status != 200:
                row[arm] = {"status": status}
                continue
            message = reply["choices"][0]["message"]
            proposed = [c["function"]["name"] for c in message.get("tool_calls") or []]
            row[arm] = {"status": status, "tools": proposed, "advertised_proposal": any(t in names for t in proposed),
                        "message_sha256": sha256(json.dumps(message, sort_keys=True).encode()).hexdigest()}
        except (URLError, TimeoutError) as exc:
            row[arm] = {"error": str(exc)}
    return row


def compare(call, endpoint, number, seed):
    body = call["recorded_request"]["body"]
    original = json.loads(body) if isinstance(body, str) else body
    edited = deepcopy(original)
    tool = edited["tools"][0]["function"]
    sentence = "Use list_databases to see available database names."
    assert tool["name"] == "SQLite_query" and tool["description"].count(sentence) == 1
    tool["description"] = tool["description"].replace(sentence, "Use SQLite_schema to inspect the attached database schema.")
    return replay_arms(call, endpoint, number, seed, {"original": original, "repeat": original, "edit": edited})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("--model-port", type=int, required=True)
    args = parser.parse_args()
    with sqlite3.connect(f"{args.database.resolve().as_uri()}?mode=ro", uri=True) as db:
        records = {}
        for (raw,) in db.execute("SELECT run FROM traces"):
            run = json.loads(raw)
            records[run["conversation_id"]] = run
    for number in range(1, 6):
        call = min(records[f"tools-{number}"]["calls"], key=lambda c: c["started_at"])
        for seed in (11, 29, 47):
            print(json.dumps(compare(call, f"http://127.0.0.1:{args.model_port}", number, seed)), flush=True)


if __name__ == "__main__":
    main()
