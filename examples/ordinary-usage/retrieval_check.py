"""Rank passages visible in earlier sessions of the frozen public PaperQA pilot."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import sqlite3
import subprocess

from run import FROZEN, ROOT, questions


def passages(run, documents, question):
    for call in sorted(run["calls"], key=lambda c: (c["started_at"], c["id"])):
        messages = json.loads(call["recorded_request"]["body"])["messages"]
        for i, message in enumerate(messages):
            content = message.get("content", "")
            if message["role"] != "user" or not content.startswith("Excerpt from "):
                continue
            heading, text = content.split("\n\n---\n\n", 1)
            text, ending = text.rsplit("\n\n---\n\n", 1)
            name = heading.removeprefix("Excerpt from ").split(": ", 1)[0]
            document = name.rsplit(" chunk ", 1)[0]
            assert ending == "Question: " + question
            offset = documents[document].index(text)
            yield name, text, {"session": run["conversation_id"], "trace": run["id"], "call": call["id"],
                               "pointer": f"/messages/{i}/content", "source": name, "offset_chars": offset,
                               "length_chars": len(text), "sha256": sha256(text.encode()).hexdigest()}


def examine(directory):
    documents = {}
    for name in ("methods", "validation", "research-review", "foundational-algorithms"):
        raw = subprocess.check_output(["git", "show", f"{FROZEN}:docs/{name}.md"], cwd=ROOT)
        documents[name] = raw.decode()
    with sqlite3.connect(f"{(directory / 'sidecar.sqlite').resolve().as_uri()}?mode=ro", uri=True) as db:
        runs = [json.loads(raw) for (raw,) in db.execute("SELECT run FROM traces")]
    runs.sort(key=lambda run: min(c["started_at"] for c in run["calls"]))
    prompts = questions("papers")
    assert len(runs) == len(prompts)
    results = []
    seen = {}
    with sqlite3.connect(":memory:") as index:
        index.execute("CREATE VIRTUAL TABLE corpus USING fts5(source UNINDEXED, passage, tokenize='porter unicode61')")
        for i, (run, prompt) in enumerate(zip(runs, prompts), 1):
            assert run["conversation_id"] == f"papers-{i}"
            current = {name: (text, ref) for name, text, ref in passages(run, documents, prompt)}
            assert len(current) == 3
            terms = list(dict.fromkeys(re.findall(r"[a-z0-9]+", prompt.lower())))
            query = " OR ".join(f'"{term}"' for term in terms)
            candidates = index.execute("SELECT source FROM corpus WHERE corpus MATCH ? ORDER BY rank, source", (query,))
            selected = next((name for (name,) in candidates if name not in current), None)
            results.append({"session": run["conversation_id"], "retrieved": sorted(current),
                            "available_earlier_alternatives": len(seen.keys() - current.keys()),
                            "candidate": seen.get(selected)})
            for name, (text, ref) in current.items():
                if name in seen:
                    assert seen[name]["sha256"] == ref["sha256"]
                else:
                    index.execute("INSERT INTO corpus VALUES (?,?)", (name, text))
                    seen[name] = ref
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    print(json.dumps(examine(parser.parse_args().directory), indent=2))
