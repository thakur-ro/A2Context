"""Cross two observed assistant messages and four catalogs at a single model decision."""
import argparse
from hashlib import sha256
from itertools import product
import json
import math
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from measure import SCOPE, cohort, first_token, prepare, user_suffix, watchlist_state
from inputs import digest
from runner import load_model, prompt_input, proposals, trial
from staging import text_hash
from replay import logger
from run import save


def reconstruct(model, tokenizer, packet, tools, cls, previous):
    audit = []
    result = trial(model, tokenizer, packet, tools, cls, dispatch_allowed=SCOPE, state_view=watchlist_state, audit=audit)
    for actual, expected in zip(result["generations"], previous["generations"], strict=True):
        assert {k: actual[k] for k in expected if k != "full_catalog_same_history_tokens"} == {
            k: v for k, v in expected.items() if k != "full_catalog_same_history_tokens"}
    assert len(audit) == 3
    for step, stored in zip(audit[:2], previous["calls"][:2], strict=True):
        cb, = step["callbacks"]
        assert (cb["name"], digest(cb["arguments"]), digest(cb["returned"]), cb["before"], cb["after"]) == (
            stored["name"], stored["arguments_sha256"], stored["feedback_sha256"], stored["before"], stored["after"])
    return audit[2]["messages"]


def decode(model, tokenizer, ids, tools):
    from mlx_lm import stream_generate
    from mlx_lm.sample_utils import make_sampler

    output = list(stream_generate(model, tokenizer, ids, max_tokens=128, sampler=make_sampler(temp=0)))
    raw = "".join(token.text for token in output)
    end = output[-1]
    row = {"raw_sha256": text_hash(raw), "generated_tokens": end.generation_tokens, "finish_reason": end.finish_reason}
    if end.finish_reason != "stop":
        return {**row, "outcome": "truncated"}
    try:
        calls = proposals(tokenizer, raw, tools)
    except (ValueError, SyntaxError, KeyError, TypeError):
        return {**row, "outcome": "parser_error"}
    exact = [(c["name"], c["arguments"]) for c in calls] == [("get_watchlist", {})]
    return {**row, "outcome": "get_watchlist" if exact else "other_proposal" if calls else "text",
            "proposals": [{"name": c["name"], "arguments_sha256": digest(c["arguments"])} for c in calls]}


def run(args):
    parent_path = HERE.parent / "results.json"
    previous = next(c for c in json.loads(parent_path.read_text())["cases"] if c["id"] == "multi_turn_base_132")
    cases, full, cls = cohort(args.upstream)
    pkt = next(p for p in cases if p["id"] == previous["id"])
    scoped = [s for s in full if s["name"] in SCOPE]
    model, tokenizer = load_model(args.snapshot)
    arms = prepare(tokenizer, pkt, full, scoped)
    history = {k: reconstruct(model, tokenizer, pkt, arms[k][1], cls, previous["arms"][k]) for k in ("full", "scoped")}
    original, edited = history["full"], history["scoped"]
    assert [m["role"] for m in original] == [m["role"] for m in edited] == ["system", "user", "assistant", "tool", "assistant", "tool"]
    assert [i for i, (a, b) in enumerate(zip(original, edited)) if a != b] == [2, 4]
    assert previous["arms"]["full"]["calls"][:2] == previous["arms"]["scoped"]["calls"][:2]
    opener, = tokenizer.encode(tokenizer.tool_call_start, add_special_tokens=False)
    report = {"task": pkt["id"], "parent_sha256": sha256(parent_path.read_bytes()).hexdigest(),
              "protocol_sha256": sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
              "source_messages_sha256": {k: [digest(m) for m in messages] for k, messages in history.items()}, "cells": []}
    for first, second in product(history, repeat=2):
        messages = list(original)
        messages[2], messages[4] = history[first][2], history[second][4]
        baseline = prompt_input(tokenizer, messages, full)
        for catalog, (variant, tools, _) in arms.items():
            current = [{"role": "system", "content": variant["system"]}, *messages[1:]]
            rendered = prompt_input(tokenizer, current, tools)
            ids = rendered[1]
            assert user_suffix(rendered) == user_suffix(baseline)
            if catalog != "scoped":
                assert len(ids) == len(baseline[1])
            prob = math.exp(float(first_token(model, ids)[opener]))
            cell = {"catalog": catalog, "first": first, "second": second, "history_sha256": digest(current),
                    "catalog_sha256": digest(tools), "prompt_sha256": text_hash(rendered[0]), "input_tokens": len(ids),
                    "opener_probability": prob, **decode(model, tokenizer, ids, tools)}
            if first == second == ("full" if catalog == "full" else "scoped"):
                saved = previous["arms"][catalog]["generations"][2]
                assert all(saved[key] == cell[key] for key in ("history_sha256", "prompt_sha256", "raw_sha256", "input_tokens", "generated_tokens"))
            report["cells"].append(cell)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("upstream", "snapshot", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--repeat", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use an unused output path.")
    logger.setLevel("ERROR")
    result = run(args)
    if args.repeat:
        if result != json.loads(args.repeat.read_text()):
            raise ValueError("Fresh process differs from the archived checkpoint.")
        result = {"report_sha256": sha256(args.repeat.read_bytes()).hexdigest(), "matched_cells": len(result["cells"])}
    save(args.output, result)
