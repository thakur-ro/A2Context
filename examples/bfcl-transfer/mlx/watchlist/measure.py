"""Measure catalog edits at matched input lengths on BFCL's watchlist simulator."""
import argparse
from hashlib import sha256
from itertools import combinations
import json
import math
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from runner import inputs, load_model, prompt_input, reference_replies, trial
from inputs import BASE, FILES, calls, digest, fetch, public_state, select
from replay import logger
from run import save

SYSTEM = "Use the available tools to complete the customer's watchlist request and report what happened."
SCOPE = {"get_watchlist", "add_to_watchlist", "remove_stock_from_watchlist", "get_stock_info", "get_symbol_by_name"}
IDS = [116, 117, 118, 126, 127, 132, 135, 142, 146]
EXTRA = {
    str(BASE / "data/multi_turn_func_doc/trading_bot.json"): "1a7933fd8f0cb8fbec38aeae05cb8c24132747cad4904fe1c13ac3d01fc22c2d",
    str(BASE / "eval_checker/multi_turn_eval/func_source_code/trading_bot.py"): "cc2e3949035b561e9d07418b081561aa07b211672200e2e511aec074bf79fcb2",
}


def cohort(root):
    # Verify shared data and namespace files before importing either simulator.
    select(root)
    for path, expected in EXTRA.items():
        if sha256((root / path).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Changed upstream file {path}")
    from bfcl_eval.eval_checker.multi_turn_eval.func_source_code.trading_bot import TradingBot

    def records(name):
        return map(json.loads, (root / BASE / "data" / name).read_text().splitlines())

    catalog = list(records("multi_turn_func_doc/trading_bot.json"))
    for tool in catalog:
        params = tool["parameters"]
        assert params["type"] == "dict"
        params["type"] = "object"
        for prop in params["properties"].values():
            if prop["type"] == "float":
                prop["type"] = "number"
    names = {tool["name"] for tool in catalog}
    assert len(names) == 20 and SCOPE <= names
    refs = {r["id"]: r["ground_truth"][0] for r in records("possible_answer/BFCL_v4_multi_turn_base.json")}
    cases = []
    for task in records("BFCL_v4_multi_turn_base.json"):
        if task["involved_classes"] != ["TradingBot"]:
            continue
        ref = calls(refs[task["id"]], names)
        methods = {name for name, _ in ref}
        if methods <= SCOPE and any("watchlist" in name for name in methods):
            messages = task["question"][0]
            assert not task.get("excluded_function") and [m["role"] for m in messages] == ["user"]
            cases.append(dict(id=task["id"], messages=messages, state=task["initial_config"]["TradingBot"],
                              ref=ref, system=SYSTEM, domain="watchlist"))
    assert [pkt["id"] for pkt in cases] == [f"multi_turn_base_{i}" for i in IDS]
    return cases, catalog, TradingBot


def watchlist_state(sim):
    # BFCL stores both integer and string order IDs. Preserve their types in JSON.
    state = public_state(sim)
    state["orders"] = sorted(sim.orders.items(), key=lambda item: (type(item[0]).__name__, item[0]))
    return state


def render(tokenizer, pkt, tools):
    return prompt_input(tokenizer, *inputs(pkt, tools))


def user_suffix(rendered):
    prompt, ids, offsets, _ = rendered
    boundary = prompt.index("<|im_start|>user\n")
    index = next(i for i, (start, _) in enumerate(offsets) if start == boundary)
    return ids[index:]


def prepare(tokenizer, pkt, full, scoped):
    baseline = render(tokenizer, pkt, full)
    short = render(tokenizer, pkt, scoped)
    arms = {"full": (pkt, full, baseline), "scoped": (pkt, scoped, short)}
    for arm, filler in [("dots", " ."), ("letters", " x")]:
        count = len(baseline[1]) - len(short[1])
        for _ in range(8):
            if count < 0:
                break
            padded = {**pkt, "system": SYSTEM + filler * count}
            rendered = render(tokenizer, padded, scoped)
            difference = len(baseline[1]) - len(rendered[1])
            if difference == 0 and user_suffix(rendered) == user_suffix(baseline):
                arms[arm] = padded, scoped, rendered
                break
            count += difference
        if arm not in arms:
            raise ValueError(f"Cannot match first-user position and length for {pkt['id']} {arm}")
    return arms


def first_token(model, ids):
    import mlx.core as mx
    import numpy as np

    logits = np.asarray(model(mx.array([ids]))[0, -1].astype(mx.float32)).astype(np.float64)
    return logits - np.logaddexp.reduce(logits)


def divergence(a, b):
    import numpy as np

    mixture = np.logaddexp(a, b) - np.log(2)
    return float((np.exp(a) @ (a - mixture) + np.exp(b) @ (b - mixture)) / (2 * np.log(2)))


def run_arm(model, tokenizer, arm, prepared, full, cls):
    pkt, visible, _ = prepared
    audit = []
    result = trial(model, tokenizer, pkt, visible, cls, dispatch_allowed=SCOPE, state_view=watchlist_state, audit=audit)
    for gen, step in zip(result["generations"], audit, strict=True):
        del gen["decode_seconds"]
        messages = [{"role": "system", "content": SYSTEM}, *step["messages"][1:]]
        full_prompt = prompt_input(tokenizer, messages, full)
        gen["full_catalog_same_history_tokens"] = len(full_prompt[1])
        if arm != "scoped" and len(full_prompt[1]) != gen["input_tokens"]:
            raise ValueError(f"Later decision length differs for {pkt['id']} {arm}")
    callbacks = [cb for step in audit for cb in step["callbacks"]]
    for event, cb in zip(result["calls"], callbacks, strict=True):
        event.update(before=cb["before"], after=cb["after"], feedback_sha256=digest(cb["returned"]))
    # A required subsequence can pass the travel endpoint. This study requires exact calls.
    result["exact_calls"] = [(e["name"], e.get("arguments_sha256")) for e in result["calls"]] == [
        (name, digest(args)) for name, args in pkt["ref"]]
    result["usable"] &= result["exact_calls"]
    return result


def measure(args):
    model, tokenizer = load_model(args.snapshot)
    cases, full, cls = cohort(args.upstream)
    scoped = [tool for tool in full if tool["name"] in SCOPE]
    opener, = tokenizer.encode(tokenizer.tool_call_start, add_special_tokens=False)
    report = {"revision": FILES["revision"], "protocol_sha256": sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
              "model_files_sha256": sha256((HERE.parent / "files.json").read_bytes()).hexdigest(),
              "catalogs_sha256": {"full": digest(full), "scoped": digest(scoped)}, "cases": []}
    prepared = []
    for pkt in cases:
        positive = trial(None, tokenizer, pkt, full, cls, dispatch_allowed=SCOPE, state_view=watchlist_state, scripted=reference_replies(tokenizer, pkt))
        negative = trial(None, tokenizer, pkt, full, cls, dispatch_allowed=SCOPE, state_view=watchlist_state, scripted=["No call."])
        assert positive["usable"] and not negative["usable"] and not negative["required_actions"] and not negative["required_responses"]
        assert [(e["name"], e["arguments_sha256"]) for e in positive["calls"]] == [(n, digest(a)) for n, a in pkt["ref"]]
        prepared.append(prepare(tokenizer, pkt, full, scoped))
        report["cases"].append({"id": pkt["id"], "positive": positive, "no_call": negative})
    print("All reference and no-call controls and both length checks passed on all nine tasks.", flush=True)
    for case, arms in zip(report["cases"], prepared, strict=True):
        scores = {arm: first_token(model, data[2][1]) for arm, data in arms.items()}
        case["opener_probability"] = {arm: math.exp(float(score[opener])) for arm, score in scores.items()}
        case["js_bits"] = {f"{a}/{b}": divergence(scores[a], scores[b]) for a, b in combinations(arms, 2)}
    for case, arms in zip(report["cases"], prepared, strict=True):
        case["arms"] = {arm: run_arm(model, tokenizer, arm, data, full, cls) for arm, data in arms.items()}
        print(case["id"], {a: r["usable"] for a, r in case["arms"].items()}, flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("snapshot", "upstream", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--repeat", type=Path)
    parser.add_argument("--fetch", action="store_true", help="Fetch the hash-pinned public BFCL files.")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use an unused output path.")
    if args.fetch:
        fetch(args.upstream)
        fetch(args.upstream, EXTRA)
    logger.setLevel("ERROR")
    result = measure(args)
    if args.repeat:
        if result != json.loads(args.repeat.read_text()):
            raise ValueError("Fresh process differs from the saved study.")
        result = {"report_sha256": sha256(args.repeat.read_bytes()).hexdigest(), "matched": [c["id"] for c in result["cases"]]}
    save(args.output, result)
