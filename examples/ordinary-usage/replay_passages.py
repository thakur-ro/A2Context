"""Run the frozen passage comparison and emit an arm-blind answer packet."""

import argparse
import asyncio
import json
from pathlib import Path
import random
import re
import secrets
import sqlite3

from paperqa import Docs, Settings

from capture import capture
from check_sidecar import server
from retrieval_check import examine
from run import questions


def settings(endpoint, seed):
    model = "openai/default_model"
    config = {"model_list": [{"model_name": model, "litellm_params": {
        "model": model, "api_base": endpoint, "api_key": "loopback", "max_tokens": 512,
        "seed": seed, "temperature": 0.5}}]}
    return Settings(llm=model, llm_config=config, summary_llm=model, summary_llm_config=config,
                    embedding="sparse", parsing={"use_doc_details": False},
                    answer={"evidence_retrieval": False, "max_concurrent_requests": 1})


async def run(args):
    ranks = examine(args.original)
    prompts = questions("papers")
    docs = Docs()
    for path in sorted(args.original.glob("*.txt")):
        await docs.aadd(path, citation=f"A2Context, {path.stem}, 2026", docname=path.stem,
                        settings=settings(f"http://127.0.0.1:{args.model_port}/v1", 11))
    passages = {text.name: text for text in docs.texts}
    args.output.mkdir(parents=True, exist_ok=False)
    answers = {}
    key = {}
    database = args.output / "captures.sqlite"
    with server(args.binary.resolve(), database, "--capture-content") as base:
        for index in (1, 3):
            rank = ranks[index]
            original = rank["retrieved"]
            prior = {source for row in ranks[:index] for source in row["retrieved"]}
            candidate = rank["candidate"]["source"]
            control = min(prior - set(original) - {candidate})
            variants = {"original": original, "repeat": original, "candidate": [*original[:-1], candidate],
                        "control": [*original[:-1], control]}
            for seed in (11, 29):
                arms = list(variants)
                random.Random(f"{rank['session']}/{seed}").shuffle(arms)
                for arm in arms:
                    token = secrets.token_hex(5)
                    sources = variants[arm]
                    subset = Docs(texts=[passages[name] for name in sources], docs=docs.docs)
                    with capture(base, args.model_port, "passage-comparison", token, args.output / "events.jsonl") as endpoint:
                        result = await subset.aquery(prompts[index], settings=settings(endpoint, seed))
                    key[token] = {"session": rank["session"], "seed": seed, "arm": arm, "sources": sources,
                                  "answer": result.answer}
                    masked = re.sub(r"(?:research-review|foundational-algorithms|methods|validation) chunk \d+",
                                    "cited source", result.answer)
                    answers[token] = {"question": prompts[index], "answer": masked}
                    (args.output / "key.json").write_text(json.dumps(key, indent=2))
    with sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True) as db:
        for (raw,) in db.execute("SELECT run FROM traces"):
            trace = json.loads(raw)
            trial = key[trace["conversation_id"]]
            assert len(trace["calls"]) == 4
            source_names = []
            for call in trace["calls"]:
                body = json.loads(call["recorded_request"]["body"])
                assert (body["seed"], body["temperature"], body["max_tokens"]) == (trial["seed"], 0.5, 512)
                assert call["decisions"]
                message = body["messages"][-1]["content"]
                if message.startswith("Excerpt from "):
                    source_names.append(message.split(": ", 1)[0].removeprefix("Excerpt from "))
            assert sorted(source_names) == sorted(trial["sources"])
        assert db.execute("SELECT count(*) FROM traces").fetchone()[0] == len(key) == 16
    (args.output / "blind.json").write_text(json.dumps(answers, indent=2, sort_keys=True))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("original", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model-port", type=int, required=True)
    asyncio.run(run(parser.parse_args()))
