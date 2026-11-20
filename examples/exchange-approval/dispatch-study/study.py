"""Run or replay the registered synthetic exchange dispatch study (Python 3.11+)."""
import argparse
from hashlib import file_digest, sha256
import json
from pathlib import Path
import sys
from time import perf_counter

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE.parents[2] / "scripts"))
import demo
from probe_transformers_answer_timeline import TransformersRuntime, load_runtime
from score_qwen_prefill import first_step, js_bits

DESIGN_SHA = "bb0d9d0da93b961aa0039e20b67716b874e8fbbd51e4eba86af6a2bde47809d4"
PROTOCOL_SHA = "b6f4fc3bb6462d315e8b27c67ec23d1fb855c6468a1c2b88ec82cfbdd198d9ee"
RUNTIME = "Transformers 4.57.6 / PyTorch 2.11.0 / CPU float32"
REVISION = "eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2"
LIMIT = 128


def study_design():
    for name, expected in (("design.json", DESIGN_SHA), ("protocol.md", PROTOCOL_SHA)):
        if sha256((HERE / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Frozen {name} changed.")
    return json.loads((HERE / "design.json").read_text())


def ordered(case, design):
    cycle = design["source_cycle"]
    i = cycle.index(case["first"])
    return cycle[i:] + cycle[:i]


def messages(case, arm, design):
    actual, stale = ("approved", "held") if case["approved"] else ("held", "approved")
    record_name = design["record_labels"][case["record"]]
    lines = [design["system"], design["lookup"], design["cache"][actual if arm == "cache" else stale],
             record_name + ": " + design["record"]["replacement" if arm == "record" else actual],
             design["control"][int(arm == "control")]]
    wording = 1 - case["wording"] if arm == "customer" else case["wording"]
    return [{"role": "system", "content": "\n".join(lines)}, {"role": "assistant", "content": design["history"]},
            {"role": "user", "content": design["customer"][actual][wording]}]


def adapter_choice(raw, stop, *, normalize_template_braces=True):
    if stop != "eos":
        return "truncated", {}
    try:
        proposal, _ = TransformersRuntime.parse_proposal(
            raw, demo.TOOLS, normalize_template_braces=normalize_template_braces)
    except ValueError:
        return "error", {}
    if proposal is None:
        return "text", {"kind": "model_output", "label": "Model answer", "outcome": raw}
    return "tool", {"kind": "tool_call", "label": proposal["name"], "outcome": proposal["arguments"]}


def trial_dispatch(choice, approved, design):
    args = design["pending_arguments"]
    checkpoint = {design["previous_order"]: "exchange_requested", args["order_id"]: "delivered"}
    return {key: demo.sandbox(choice, approved, enforce=gate, checkpoint=checkpoint, expected_arguments=args)
            for key, gate in (("without_gate", False), ("with_gate", True))}


def generate_tokens(rt, ids, limit):
    inputs = rt.torch.tensor([ids])
    generated = rt.model.generate(
        input_ids=inputs, attention_mask=rt.torch.ones_like(inputs),
        generation_config=rt.generation_config, max_new_tokens=limit, use_cache=True,
    )
    return generated[0, len(ids):].tolist()


def generate(rt, ids):
    start = perf_counter()
    output = generate_tokens(rt, ids, LIMIT)
    eos = output[-1] in rt.generation_config.eos_token_id
    raw = rt.tokenizer.decode(output[:-1] if eos else output, skip_special_tokens=False)
    return {"raw": raw, "stop": "eos" if eos else "token_limit", "generated_tokens": len(output),
            "decode_seconds": perf_counter() - start}


def measure(rt, case, design):
    order = ordered(case, design)
    arms, token_ids = {}, {}
    for name in ["original", *order]:
        start = perf_counter()
        prompt, ids, _, _ = rt.prompt_input(messages(case, name, design), demo.TOOLS)
        token_ids[name] = ids
        logps = first_step(rt, ids)
        if name == "original":
            baseline = logps
        arms[name] = {"prompt_sha256": sha256(prompt.encode()).hexdigest(), "input_tokens": len(ids),
                      "js_bits": 0.0 if name == "original" else js_bits(rt.torch, baseline, logps),
                      "probe_seconds": perf_counter() - start}
    ranked = sorted(order, key=lambda n: -arms[n]["js_bits"])
    for name, arm in arms.items():
        arm.update(generate(rt, token_ids[name]))
        arm["parse"], arm["choice"] = adapter_choice(arm["raw"], arm["stop"])
        arm.update(trial_dispatch(arm["choice"], case["approved"], design))
    return {"id": case["id"], "approved": case["approved"], "order": order, "ranked": ranked, "arms": arms}


def compare(cases, design):
    result = {}
    oid = design["pending_arguments"]["order_id"]
    for gate in ("without_gate", "with_gate"):
        steps = []
        for case in cases:
            states = {k: arm[gate]["orders"][oid] for k, arm in case["arms"].items()}
            original = states["original"]
            expected = "exchange_requested" if case["approved"] else "delivered"
            row = {"id": case["id"], "original_state": original, "expected_state": expected}
            for label, order in (("ranked", case["ranked"]), ("listed", case["order"])):
                row[label + "_first_change"] = next((i for i, k in enumerate(order, 1) if states[k] != original), None)
                top = states[order[0]]
                row[label + "_top"] = "unchanged" if top == original else "restored" if top == expected else "lost"
            steps.append(row)
        result[gate] = steps
    return result


def check(saved, design):
    if saved["design_sha256"] != DESIGN_SHA or saved["protocol_sha256"] != PROTOCOL_SHA:
        raise ValueError("Unexpected frozen design.")
    if [case["id"] for case in saved["cases"]] != [c["id"] for c in design["cases"]]:
        raise ValueError("Case IDs or ordering differ.")
    for row, case in zip(saved["cases"], design["cases"]):
        order = ordered(case, design)
        arms = row["arms"]
        if (case["approved"] != row["approved"] or order != row["order"] or set(arms) != {"original", *order}
                or sorted(order, key=lambda k: -arms[k]["js_bits"]) != row["ranked"]):
            raise ValueError("Ranking or arms disagree with saved probes.")
        for arm in arms.values():
            if (arm["stop"] not in ("eos", "token_limit") or not 1 <= arm["generated_tokens"] <= LIMIT
                    or (arm["stop"] == "token_limit" and arm["generated_tokens"] != LIMIT)
                    or not 0 <= arm["js_bits"] <= 1 or arm["input_tokens"] < 1):
                raise ValueError("Impossible score, token count or stop condition.")
            parse, choice = adapter_choice(arm["raw"], arm["stop"])
            if arm["parse"] != parse or arm["choice"] != choice:
                raise ValueError("Saved choice differs from the adapter.")
            for gate, receipt in trial_dispatch(choice, case["approved"], design).items():
                if arm[gate] != receipt or receipt["orders"][design["previous_order"]] != "exchange_requested":
                    raise ValueError("State transition differs from the saved SQLite receipt.")
    if compare(saved["cases"], design) != saved["comparison"]:
        raise ValueError("The first-edit comparison differs from the saved dispatches.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    design = study_design()
    if args.check:
        if args.snapshot or args.output:
            parser.error("--check takes only the results path")
        data = json.loads(args.check.read_text())
        check(data, design)
        print(f"Verified {len(data['cases'])} rankings and all 80 SQLite replays.")
        return
    if not args.snapshot or not args.output or args.output.exists():
        parser.error("Provide --snapshot and a new --output path.")
    demo.verify_checkpoint(args.snapshot)
    for name, expected in (
        ("config.json", "18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45"),
        ("generation_config.json", "e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6"),
        ("tokenizer_config.json", "006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680"),
    ):
        with (args.snapshot / name).open("rb") as f:
            if file_digest(f, "sha256").hexdigest() != expected:
                raise ValueError(f"Pinned snapshot differs for {name}")
    runtime = load_runtime(args.snapshot)
    if runtime.identity != RUNTIME:
        raise ValueError("Pinned runtime required.")
    with runtime.torch.inference_mode():
        cases = [measure(runtime, c, design) for c in design["cases"]]
    data = {"model": demo.MODEL, "revision": REVISION, "runtime": RUNTIME, "design_sha256": DESIGN_SHA,
            "protocol_sha256": PROTOCOL_SHA, "cases": cases, "comparison": compare(cases, design)}
    check(data, design)
    with args.output.open("x") as out:
        json.dump(data, out, indent=2, ensure_ascii=False, allow_nan=False)
        out.write("\n")


if __name__ == "__main__":
    main()
