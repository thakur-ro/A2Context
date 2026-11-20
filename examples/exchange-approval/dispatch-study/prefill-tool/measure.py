"""Generate two assistant continuations and replay their exact strings through Qwen-Agent and SQLite."""
import argparse
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import sys
from time import perf_counter

from freeze import HERE, LIMIT, PREFIX, design, digest
sys.path.insert(0, str(HERE.parent / "prospective"))
from run import (DESIGN, GATES, MODEL_FILES, REVISION, RUNTIME, attempt, check_known_calls, demo, effect, expected,
                 generate_tokens, instrumentation, load_runtime, logger, version, workflow)
from replay import replay

DESIGN_SHA = "0d4200e113c7fd6ffa74a70567874f1f0dceeebdf2bec1b2ec66f14fd679d6e1"


def load_inputs(tokenizer):
    raw = (HERE / "design.json").read_bytes()
    spec = json.loads(raw)
    source = (HERE.parent / "prospective/results.json").read_bytes()
    rendered = (HERE.parent / "rendering/results.json").read_bytes()
    if (sha256(raw).hexdigest() != DESIGN_SHA or sha256(source).hexdigest() != spec["parent_sha256"]
            or sha256(rendered).hexdigest() != spec["render_results_sha256"]
            or sha256((HERE / "protocol.md").read_bytes()).hexdigest() != spec["protocol_sha256"]):
        raise ValueError("Frozen inputs or protocol changed.")
    if tokenizer and design(tokenizer) != spec:
        raise ValueError("The tokenizer did not reproduce the full frozen prompt pairs.")
    return spec, json.loads(source)


def generate_or_verify(runtime, tokenizer, prompt, prefix, prefix_ids, row, saved):
    if digest(prompt + prefix) != row["prompt_sha256"]:
        raise ValueError("Rendered string differs from the registered assistant boundary.")
    if tokenizer:
        ids = tokenizer.encode(prompt + prefix, add_special_tokens=False)
        if digest(ids) != row["tokens_sha256"] or len(ids) != row["input_tokens"]:
            raise ValueError("Input IDs differ from the frozen prefix.")
    limit = LIMIT - len(prefix_ids)
    if runtime:
        start = perf_counter()
        with runtime.torch.inference_mode():
            output = generate_tokens(runtime, ids, limit)
        seconds = perf_counter() - start
        eos = output[-1] in runtime.generation_config.eos_token_id
        suffix = tokenizer.decode(output[:-1] if eos else output, skip_special_tokens=False)
        g = {"model_output_ids": output, "generated_suffix": suffix, "parser_input": prefix + suffix,
             "stop": "eos" if eos else "token_limit", "decode_seconds": seconds}
    else:
        g = saved
    output, suffix, raw = g["model_output_ids"], g["generated_suffix"], g["parser_input"]
    if (raw != prefix + suffix or not 1 <= len(output) <= limit or g["stop"] not in ("eos", "token_limit")
            or (g["stop"] == "token_limit" and len(output) != limit)):
        raise ValueError("Inconsistent supplied prefix, suffix or total assistant budget.")
    if tokenizer:
        eos = output[-1] in (tokenizer.eos_token_id, tokenizer.convert_tokens_to_ids("<|endoftext|>"))
        body = output[:-1] if eos else output
        if ((g["stop"] == "eos") != eos or tokenizer.decode(body, skip_special_tokens=False) != suffix
                or tokenizer.decode(prefix_ids + body, skip_special_tokens=False) != raw):
            raise ValueError("The supplied and generated token IDs do not reconstruct the parser input.")
    return g


def through_framework(case, consent, parent, g, prefix_ids, tracer, tokenizer):
    # Replay transport sees the original request and delivers the complete assistant message in one chunk.
    prior = parent["generation"]
    native = {"pending_request": parent["pending_request"],
              "generation": {"rendered_prompt": prior["rendered_prompt"], "prompt_sha256": prior["prompt_sha256"],
                             "prompt_tokens_sha256": prior["prompt_tokens_sha256"], "input_tokens": prior["input_tokens"],
                             "raw": g["parser_input"], "raw_sha256": digest(g["parser_input"]), "stop": g["stop"],
                             "output_ids": prefix_ids + g["model_output_ids"]}}
    gated = {}
    for gate, enforce in GATES.items():
        result, gated[gate] = attempt(case, consent, tracer, enforce, saved=native, tokenizer=tokenizer)
        if (result["workflow_request"] != parent["workflow_request"] or result["pending_request"] != parent["pending_request"]
                or result["generation"]["raw"] != g["parser_input"]):
            raise ValueError("Native replay diverged from the registered input or reconstructed parser string.")
        if gate == "without_gate":
            parser = result["parser"]
        elif parser != result["parser"]:
            raise ValueError("Approval enforcement changed the parser outcome.")
    return {"generation": g, "parser": parser, **gated}


def stable(arm):
    return {**arm, "generation": {k: v for k, v in arm["generation"].items() if k != "decode_seconds"}}


def summarize(rows, spec):
    scores, costs = {}, {}
    for name in ("empty", "prefilled"):
        score, cost = Counter(), Counter()
        for pair, row in zip(spec["pairs"], rows, strict=True):
            arm = row[name]
            parser, g = arm["parser"], arm["generation"]
            score[parser["disposition"]] += 1
            cost.update(calls=1, input_tokens=pair[name]["input_tokens"], model_output_tokens=len(g["model_output_ids"]),
                        supplied_assistant_tokens=len(spec["prefix_ids"]) if name == "prefilled" else 0,
                        decode_seconds=g["decode_seconds"])
            if g["stop"] == "eos":
                score.update(scorable=1, expected_tool=int((parser["call"] or {}).get("name") == row["expected"]["tool"]),
                             native_validated=int(parser["validated_choice"] is not None),
                             exact_expected_call=int(parser["exact_expected_call"]))
                for gate in GATES:
                    score[gate + "_expected_state"] += int(arm[gate]["expected_state"])
        scores[name], costs[name] = score, cost
    paired = {gate: Counter() for gate in GATES}
    for row in rows:
        scorable = all(row[k]["generation"]["stop"] == "eos" for k in ("empty", "prefilled"))
        for gate in GATES:
            change = effect(row["empty"][gate]["expected_state"], row["prefilled"][gate]["expected_state"])
            paired[gate][change if scorable else "unscorable"] += 1
    return {"scores": scores, "cost": costs, "paired": paired}


def run(runtime=None, tokenizer=None, published=None, repeat=False):
    spec, prior = load_inputs(tokenizer)
    rows = []
    with instrumentation() as tracer:
        for i, pair in enumerate(spec["pairs"]):
            case = DESIGN[i // 2]
            consent = pair["consent_arm"]
            if pair["case"] != case["id"] or consent != ("original", "repair")[i % 2]:
                raise ValueError("Frozen case order differs.")
            parent = prior["cases"][i // 2]["arms"][consent]
            row = {"case": case["id"], "consent_arm": consent, "expected": expected(case, demo.WRITE)}
            for name in (("prefilled",) if repeat else ("empty", "prefilled")):
                prefix = PREFIX if name == "prefilled" else ""
                prefix_ids = spec["prefix_ids"] if prefix else []
                saved = published["pairs"][i][name] if published else None
                g = generate_or_verify(runtime, tokenizer, parent["generation"]["rendered_prompt"], prefix, prefix_ids,
                                       pair[name], saved["generation"] if saved else None)
                arm = through_framework(case, consent, parent, g, prefix_ids, tracer, tokenizer)
                if name == "empty":
                    if (g["model_output_ids"] != parent["generation"]["output_ids"] or g["parser_input"] != parent["generation"]["raw"]
                            or any(arm[k] != parent[k] for k in ("parser", *GATES))):
                        raise ValueError("The empty-prefix null failed to reproduce a prior completion or receipt.")
                if saved and stable(saved) != stable(arm):
                    raise ValueError("A saved or independently repeated outcome differs.")
                row[name] = arm
            rows.append(row)
    if repeat:
        gs = [row["prefilled"]["generation"] for row in rows]
        return {"reproduced_pairs": len(gs), "input_tokens": sum(p["prefilled"]["input_tokens"] for p in spec["pairs"]),
                "model_output_tokens": sum(len(g["model_output_ids"]) for g in gs),
                "supplied_assistant_tokens": len(gs) * len(spec["prefix_ids"]),
                "decode_seconds": sum(g["decode_seconds"] for g in gs)}
    return {"design_sha256": DESIGN_SHA, "model": demo.MODEL, "revision": REVISION, "runtime": RUNTIME,
            "qwen_agent": version("qwen-agent"), "pairs": rows, "summary": summarize(rows, spec)}


def controls():
    check_known_calls()
    with instrumentation() as tracer:
        for case in DESIGN[:2]:
            request, _ = workflow(case, "original", demo.TOOLS, demo.MODEL)
            checkpoint = {"pending_arguments": case["pending_arguments"], "previous_order": case["previous_order"]}
            for enforce in GATES.values():
                trial = replay("No exchange was submitted.", request["messages"], case["approved"], enforce,
                               checkpoint, tracer, case["id"])
                if trial["disposition"] != "text" or trial["receipt"]["dispatch"] != "no_write":
                    raise ValueError("Text null unexpectedly triggered a call or write.")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--check", type=Path)
    mode.add_argument("--repeat", type=Path)
    p.add_argument("--snapshot", type=Path)
    p.add_argument("--output", type=Path)
    args = p.parse_args()
    if version("qwen-agent") != "0.0.34":
        p.error("Use the registered Qwen-Agent version.")
    saved = json.loads((args.check or args.repeat).read_text()) if args.check or args.repeat else None
    if args.check:
        if args.output:
            p.error("--check accepts only an optional tokenizer snapshot.")
        tokenizer = None
        if args.snapshot:
            from transformers import AutoTokenizer
            tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
        if run(tokenizer=tokenizer, published=saved) != saved:
            raise ValueError("Saved paired report differs from the reconstructed results.")
        controls()
        print("Verified 16 parser inputs, both SQLite policies, four known calls and four plain-text nulls.")
        return
    if not args.snapshot or not args.output or args.output.exists():
        p.error("Fresh inference needs a local snapshot and unused output path.")
    demo.verify_checkpoint(args.snapshot)
    if any(sha256((args.snapshot / name).read_bytes()).hexdigest() != sha for name, sha in MODEL_FILES.items()):
        p.error("Unexpected model file.")
    rt = load_runtime(args.snapshot)
    if rt.identity != RUNTIME:
        p.error("Runtime differs.")
    data = run(rt, rt.tokenizer, saved, bool(args.repeat))
    if args.repeat:
        data["results_sha256"] = sha256(args.repeat.read_bytes()).hexdigest()
    with args.output.open("x") as f:
        json.dump(data, f, ensure_ascii=False, indent=2, allow_nan=False)
        f.write("\n")


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
