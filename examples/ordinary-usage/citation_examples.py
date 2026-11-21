"""Replay the frozen PaperQA citation-key edit on a local model."""

import argparse
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import random
import re
import sqlite3
import sys
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_sidecar import send

KEY = re.compile(r"\bpqac-[a-f0-9]{8}\b")


def run_trials(run, endpoint):
    call = max(run["calls"], key=lambda c: c["started_at"])
    original = json.loads(call["recorded_request"]["body"])
    user = original["messages"][-1]["content"]
    before, boundary, instructions = user.partition("Write an answer based on the context.")
    assert boundary
    passages = sorted(set(re.findall(r"^(pqac-[a-f0-9]{8}):", before, re.MULTILINE)))
    examples = sorted(set(KEY.findall(instructions)) - set(passages))
    assert len(passages) == 3 and len(examples) == 2 and all(key not in before for key in examples)
    swaps = {key: "pqac-" + sha256((run["id"] + key).encode()).hexdigest()[:8] for key in examples}
    assert len(set(swaps.values())) == len(swaps) and all(v not in json.dumps(original) for v in swaps.values())
    edited = deepcopy(original)
    edited["messages"][-1]["content"] = before + boundary + KEY.sub(lambda m: swaps.get(m[0], m[0]), instructions)
    for seed in (11, 29, 47):
        arms = ["original", "repeat", "edit"]
        random.Random(f"{run['id']}/{seed}").shuffle(arms)
        row = {"session":run["conversation_id"], "call":call["id"], "seed":seed, "order":arms,
               "passages": passages, "swaps": swaps}
        for arm in arms:
            request = dict(edited if arm == "edit" else original, temperature=0.5, seed=seed, max_tokens=512)
            try:
                status, reply = send(endpoint, "/v1/chat/completions", request, expected=None)
                if status != 200:
                    row[arm] = {"http":status}
                    continue
                answer = reply["choices"][0]["message"]["content"]
                cited = {key for group in re.findall(r"\(([^()\n]*)\)", answer) for key in KEY.findall(group)}
                row[arm] = {"http":status, "citations":sorted(cited), "answer_sha256":sha256(answer.encode()).hexdigest()}
            except (TimeoutError, URLError) as exc:
                row[arm] = {"error":str(exc)}
        yield row


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("--model-port", type=int, required=True)
    args = parser.parse_args()
    with sqlite3.connect(f"{args.database.resolve().as_uri()}?mode=ro", uri=True) as db:
        runs = [json.loads(raw) for raw, in db.execute("SELECT run FROM traces")]
    for run in sorted(runs, key=lambda r: r["conversation_id"]):
        for row in run_trials(run, f"http://127.0.0.1:{args.model_port}"):
            print(json.dumps(row), flush=True)
