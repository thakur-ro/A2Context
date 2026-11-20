"""Measure or verify the frozen renderer pairs through the same native parser and SQLite gates."""
import argparse
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path

from renderers import DOUBLE, HERE, SINGLE, TokenizerTools, manifest, tokenizer_request
from run import (DESIGN, GATES, MODEL_FILES, REVISION, RUNTIME, LocalGeneration, attempt, check_known_calls,
                 demo, effect, expected, instrumentation, load_runtime, logger, pending, version, workflow)

RENDERERS = {"nous": LocalGeneration, "tokenizer": TokenizerTools}
DESIGN_SHA = "ff83eae2a7072adf9a9e2da60a043214af55c17d713b2e1cf967f4b461a2ae0b"


def read_sources(tokenizer):
    frozen = (HERE / "design.json").read_bytes()
    if sha256(frozen).hexdigest() != DESIGN_SHA:
        raise ValueError("The registered pair manifest changed.")
    design = json.loads(frozen)
    previous = (HERE.parent / "prospective/results.json").read_bytes()
    for key, raw in (("previous_results_sha256", previous), ("protocol_sha256", (HERE / "protocol.md").read_bytes()),
                     ("renderers_sha256", (HERE / "renderers.py").read_bytes())):
        if sha256(raw).hexdigest() != design[key]:
            raise ValueError(f"Source digest changed for {key}")
    parent = json.loads(previous)
    if tokenizer and manifest(parent, tokenizer) != design["pairs"]:
        raise ValueError("The current tokenizer did not reproduce the complete registered render manifest.")
    return design, parent


def reference_input(case, consent, parent, renderer):
    request, cache = workflow(case, consent, demo.TOOLS, demo.MODEL)
    if request != parent["workflow_request"]:
        raise ValueError("Application input differs from the original study.")
    original = parent["pending_request"]
    prompt = parent["generation"]["rendered_prompt"]
    if renderer == "nous":
        return original, prompt
    history = json.loads(original["attributes"]["nexusweave.request.body"])["messages"]
    messages, tools = tokenizer_request(request, history)
    return pending(case, consent, messages, cache, tools), prompt.replace(SINGLE, DOUBLE)


def stable(trial):
    return {**trial, "generation": {k: v for k, v in trial["generation"].items() if k != "decode_seconds"}}


def measure(case, consent, parent, renderer, spec, tracer, runtime, tokenizer, prior):
    boundary, prompt = reference_input(case, consent, parent, renderer)
    transport = RENDERERS[renderer]
    replay_input = None if runtime else {"pending_request": boundary,
                                         "generation": {**prior["generation"], "rendered_prompt": prompt}}
    first, ungated = attempt(case, consent, tracer, False, runtime, replay_input, tokenizer, transport_type=transport)
    second, gated = attempt(case, consent, tracer, True, saved=first, tokenizer=tokenizer, transport_type=transport)
    gen = first["generation"]
    if (first != second or first["pending_request"] != boundary or first["workflow_request"] != parent["workflow_request"]
            or gen["rendered_prompt"] != prompt or any(gen[key] != value for key, value in spec[renderer].items())):
        raise ValueError("Registered rendering, input tokens, parser or boundary differed between gates.")
    result = {"generation": {k: v for k, v in gen.items() if k != "rendered_prompt"},
              "parser": first["parser"], "without_gate": ungated, "with_gate": gated}
    if renderer == "nous":
        a = {**result, "generation": gen}
        reference = {k: parent[k] for k in a}
        if stable(a) != stable(reference):
            raise ValueError("Fresh Nous result differs from the preceding exact prompt, output or receipts.")
    if prior and stable(result) != stable(prior):
        raise ValueError("Renderer result differs from saved output or native receipts.")
    return result


def summarize(rows):
    scores = {renderer: Counter() for renderer in RENDERERS}
    costs = {renderer: Counter() for renderer in RENDERERS}
    effects = {gate: Counter() for gate in GATES}
    wording = 0
    for row in rows:
        for renderer in RENDERERS:
            arm = row[renderer]
            parser = arm["parser"]
            g = arm["generation"]
            cost = costs[renderer]
            cost.update(calls=1, input_tokens=g["input_tokens"], output_tokens=len(g["output_ids"]),
                        decode_seconds=g["decode_seconds"])
            scores[renderer][parser["disposition"]] += 1
            if g["stop"] == "eos":
                name = (parser["call"] or {}).get("name")
                scores[renderer].update(complete=1, expected_name=int(name == row["expected"]["tool"]),
                                        validated=int(parser["validated_choice"] is not None),
                                        exact_expected_call=int(parser["exact_expected_call"]))
                for gate in GATES:
                    scores[renderer][gate + "_expected_state"] += int(arm[gate]["expected_state"])
        complete = all(row[r]["generation"]["stop"] == "eos" for r in RENDERERS)
        for gate in GATES:
            label = effect(row["nous"][gate]["expected_state"], row["tokenizer"][gate]["expected_state"])
            effects[gate][label if complete else "unscorable"] += 1
        if complete:
            wording += row["nous"]["generation"]["raw"] != row["tokenizer"]["generation"]["raw"]
    return {"scores": scores, "paired_state_effects": effects, "different_completed_strings": wording, "cost": costs}


def run(tokenizer, runtime=None, published=None, repeat=False):
    design, source = read_sources(tokenizer)
    rows = []
    with instrumentation() as tracer:
        for i, spec in enumerate(design["pairs"]):
            case, original = DESIGN[i // 2], source["cases"][i // 2]
            consent = spec["consent_arm"]
            if spec["case"] != case["id"] or case["id"] != original["id"] or consent != ("original", "repair")[i % 2]:
                raise ValueError("Original study and frozen pair identities disagree.")
            row = {"case": case["id"], "consent_arm": consent, "expected": expected(case, demo.WRITE)}
            for renderer in (("tokenizer",) if repeat else RENDERERS):
                prior = published["pairs"][i][renderer] if published else None
                row[renderer] = measure(case, consent, original["arms"][consent], renderer, spec, tracer,
                                        runtime, tokenizer, prior)
            rows.append(row)
    if repeat:
        generations = [row["tokenizer"]["generation"] for row in rows]
        return {"matched_pairs": [f"{row['case']}/{row['consent_arm']}" for row in rows],
                "calls": len(rows), "input_tokens": sum(g["input_tokens"] for g in generations),
                "output_tokens": sum(len(g["output_ids"]) for g in generations),
                "decode_seconds": sum(g["decode_seconds"] for g in generations)}
    return {"design_sha256": sha256((HERE / "design.json").read_bytes()).hexdigest(),
            "model": demo.MODEL, "revision": REVISION, "runtime": RUNTIME, "qwen_agent": version("qwen-agent"),
            "pairs": rows, "summary": summarize(rows)}


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
            p.error("--check accepts only an optional --snapshot.")
        tokenizer = None
        if args.snapshot:
            from transformers import AutoTokenizer
            tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
        if run(tokenizer, published=saved) != saved:
            raise ValueError("Published counts or receipts differ.")
        check_known_calls()
        print("Verified 16 generations, four known-call plumbing controls and all paired receipts.")
        return
    if not args.snapshot or not args.output or args.output.exists():
        p.error("Supply the pinned --snapshot and an unused --output file.")
    demo.verify_checkpoint(args.snapshot)
    for name, expected_hash in MODEL_FILES.items():
        if sha256((args.snapshot / name).read_bytes()).hexdigest() != expected_hash:
            p.error(f"Unexpected model file {name}")
    rt = load_runtime(args.snapshot)
    if rt.identity != RUNTIME:
        p.error("The local inference runtime differs.")
    data = run(rt.tokenizer, rt, saved, bool(args.repeat))
    if args.repeat:
        data["results_sha256"] = sha256(args.repeat.read_bytes()).hexdigest()
    with args.output.open("x") as output:
        json.dump(data, output, ensure_ascii=False, indent=2, allow_nan=False)
        output.write("\n")


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
