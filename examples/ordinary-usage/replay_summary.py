"""Replace one summary in each recorded candidate-arm final request."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import random
import secrets
import sqlite3
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_sidecar import send


def compare(run, endpoint):
    excerpts = {}
    summaries = {}
    final = None
    for call in run["calls"]:
        body = json.loads(call["recorded_request"]["body"])
        message = body["messages"][-1]["content"]
        if not message.startswith("Excerpt from "):
            assert final is None
            final = body
            continue
        heading, rest = message.split("\n\n---\n\n", 1)
        name = heading.removeprefix("Excerpt from ").split(": ", 1)[0]
        excerpts[name] = rest.rsplit("\n\n---\n\n", 1)[0]
        summaries[name] = call["decisions"][0]["outcome"]
    source = "foundational-algorithms chunk 2"
    summary = json.loads(summaries[source])["summary"]
    other = min(name for name in excerpts if name != source)
    assert final["messages"][-1]["content"].count(summary) == 1
    variants = {"original": summary, "repeat": summary, "passage": excerpts[source], "control": excerpts[other]}
    order = list(variants)
    random.Random(final["seed"]).shuffle(order)
    for arm in order:
        body = deepcopy(final)
        message = body["messages"][-1]
        message["content"] = message["content"].replace(summary, variants[arm])
        status, reply = send(endpoint, "/v1/chat/completions", body, expected=None)
        yield {"arm": arm, "seed": final["seed"], "replacement_chars": len(variants[arm]),
               "status": status, "answer": reply["choices"][0]["message"]["content"] if status == 200 else None}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trials", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--model-port", required=True, type=int)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    keys = json.loads((args.trials / "key.json").read_text())
    outcomes = {}
    dbpath = (args.trials / "captures.sqlite").resolve().as_uri()
    with sqlite3.connect(f"{dbpath}?mode=ro", uri=True) as db:
        for (raw,) in db.execute("SELECT run FROM traces"):
            run = json.loads(raw)
            key = keys[run["conversation_id"]]
            if (key["session"], key["arm"]) != ("papers-4", "candidate"):
                continue
            for result in compare(run, f"http://127.0.0.1:{args.model_port}"):
                outcomes[secrets.token_hex(5)] = result
    assert len(outcomes) == 8
    (args.output / "key.json").write_text(json.dumps(outcomes, indent=2))
    (args.output / "blind.json").write_text(json.dumps({k: v["answer"] for k, v in outcomes.items()},
                                                       indent=2, sort_keys=True))
