"""Replay the registered LlamaIndex worked-example edit using a local model."""

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

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts"))
from check_sidecar import send

WORDS = {"water":"wax", "sky":"lamp", "wet":"soft"}
TOPIC = re.compile(r"\b(water|sky|wax|lamp)\b", re.I)


def replace_word(match):
    word = match.group()
    new = WORDS[word.lower()]
    return new.capitalize() if word[0].isupper() else new


def run_trials(run, endpoint):
    call = max(run["calls"], key=lambda c: c["started_at"])
    body = json.loads(call["recorded_request"]["body"])
    assert len(body["messages"]) == 1
    prompt = body["messages"][0]["content"]
    example, boundary, rest = prompt.partition("Now it's your turn.")
    assert boundary and "For example:" in example and "Source 1:" in rest
    if TOPIC.search(rest):
        yield {"session":run["conversation_id"],"call":call["id"],"excluded":"Topic terms also occur after the example."}
        return
    instructions, start, demonstration = example.partition("For example:")
    assert start and not TOPIC.search(instructions)
    variant = deepcopy(body)
    variant["messages"][0]["content"] = instructions + start + re.sub(
        r"\b(?:water|sky|wet)\b", replace_word, demonstration, flags=re.I) + boundary + rest
    for seed in (11, 29, 47):
        arms = ["original", "repeat", "edit"]
        random.Random(f"{run['id']}/{seed}").shuffle(arms)
        trial = {"session":run["conversation_id"],"call":call["id"],"seed":seed,"order":arms}
        for arm in arms:
            request = dict(variant if arm == "edit" else body, seed=seed, temperature=0.5, max_tokens=512)
            try:
                status, data = send(endpoint, "/v1/chat/completions", request, expected=None)
                result = {"http":status}
                if status == 200:
                    text = data["choices"][0]["message"]["content"]
                    matches = list(TOPIC.finditer(text))
                    result.update(terms=sorted({m[0].lower() for m in matches}), answer_sha256=sha256(text.encode()).hexdigest(),
                                  excerpts=[text[max(0,m.start()-36):m.end()+36] for m in matches[:4]])
            except (URLError, TimeoutError) as exc:
                result = {"error":str(exc)}
            trial[arm] = result
        yield trial


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    with sqlite3.connect(args.database.resolve().as_uri()+"?mode=ro", uri=True) as db:
        for raw, in db.execute("SELECT run FROM traces ORDER BY thread"):
            for trial in run_trials(json.loads(raw), f"http://127.0.0.1:{args.port}"):
                print(json.dumps(trial), flush=True)
