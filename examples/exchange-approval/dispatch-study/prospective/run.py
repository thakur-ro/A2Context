"""Generate, replay or independently repeat the registered prospective exchange cases."""
import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path
import sys
from time import perf_counter

from scenarios import DESIGN, HERE, expected, workflow

sys.path.insert(0, str(HERE.parent / "framework"))
from replay import FnCallAgent, LocalTool, Message, SavedCompletion, demo, instrumentation, logger, replay
from study import LIMIT, REVISION, RUNTIME, generate_tokens, load_runtime

FROZEN = {
    "design.json": "ab8faa7464e9e5e6548fbf314f5f810c5e5f9a98da1a90bbd8936a7c73eae55c",
    "scenarios.py": "4433d63f0f09a6b498f0d2b832d5b3f7f67f9e6f213f8ce164d6e01886ac23ea",
    "protocol.md": "51902b32c8faad00fe74aac11d9d0b087963846da698ce983bcb0d155c3ae40a",
}
GATES = {"without_gate": False, "with_gate": True}
MODEL_FILES = {
    "config.json": "18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45",
    "generation_config.json": "e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6",
    "tokenizer_config.json": "006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680",
}


def digest(value):
    if not isinstance(value, str):
        value = json.dumps(value, separators=(",", ":"), ensure_ascii=False)
    return sha256(value.encode()).hexdigest()


def pending(case, arm, history, cache, tools=None):
    body = {"model": demo.MODEL, "messages": history, "temperature": 0, "max_tokens": LIMIT}
    if tools is not None:
        body["tools"] = tools
    sources = []
    for name, role, text, consent in [
        ("confirmation-cache", "system", cache, case["approved"] if arm == "repair" else not case["approved"]),
        ("latest-customer-message", "user", case["utterance"], case["approved"]),
    ]:
        matches = [(i, m["content"]) for i, m in enumerate(history) if m["role"] == role and text in m["content"]]
        (index, content), = matches
        if content.count(text) != 1:
            raise ValueError(f"Ambiguous source fragment for {name}")
        start = len(content.partition(text)[0].encode())
        sources.append({"id": name, "source_id": name, "source_version": "v2" if name == "confirmation-cache"
                        and arm == "repair" else "v1", "role": role,
                        "request_fragment": demo.fragment(f"/messages/{index}/content", text, start),
                        "rule": {"key": "exchange_authorization", "scope": case["pending_arguments"]["order_id"],
                                 "value": "approved" if consent else "hold"}})
    return {"version": 1, "attributes": {"gen_ai.request.model": demo.MODEL,
            "a2context.organization.id": "exchange-study", "service.name": "exchange-staging",
            "a2context.request.body": json.dumps(body, ensure_ascii=False, separators=(",", ":")),
            "a2context.context.semantics": {"version": 1, "request_sources": sources}}}


class IncompleteOutput(Exception):
    pass


class LocalGeneration(SavedCompletion):
    """Capture exactly what reaches the tokenizer after native Nous preprocessing."""
    def __init__(self, case, arm, cache, tracer, runtime, saved, tokenizer):
        super().__init__(None, tracer, case["id"])
        self.case, self.arm, self.cache_text = case, arm, cache
        self.runtime, self.saved, self.tokenizer = runtime, saved, tokenizer
        self.boundary = self.generation = None

    def model_inputs(self, history):
        return history, None

    def _chat_stream(self, messages, delta_stream, generate_cfg):
        self.emissions += 1
        if self.emissions != 1:
            raise ValueError("Only one completion is registered.")
        history = [{"role": msg.role, "content": msg.content} for msg in messages]
        if any(not isinstance(msg["content"], str) for msg in history):
            raise ValueError("Expected plain text history after Nous preprocessing.")
        history, tools = self.model_inputs(history)
        self.boundary = pending(self.case, self.arm, history, self.cache_text, tools)
        if self.saved and self.boundary != self.saved["pending_request"]:
            raise ValueError("The reconstructed model request differs from the recorded boundary.")
        if self.runtime:
            prompt, ids, _, _ = self.runtime.prompt_input(history, tools)
            start = perf_counter()
            with self.runtime.torch.inference_mode():
                output = generate_tokens(self.runtime, ids, LIMIT)
            elapsed = perf_counter() - start
            eos = output[-1] in self.runtime.generation_config.eos_token_id
            raw = self.runtime.tokenizer.decode(output[:-1] if eos else output, skip_special_tokens=False)
            self.generation = {"rendered_prompt": prompt, "prompt_sha256": digest(prompt), "prompt_tokens_sha256": digest(ids),
                               "input_tokens": len(ids), "raw": raw, "raw_sha256": digest(raw),
                               "output_ids": output, "stop": "eos" if eos else "token_limit", "decode_seconds": elapsed}
        else:
            self.generation = self.saved["generation"]
            if self.tokenizer:
                prompt = self.tokenizer.apply_chat_template(history, tools=tools, tokenize=False, add_generation_prompt=True)
                ids = self.tokenizer.encode(prompt, add_special_tokens=False)
                g = self.generation
                output = g["output_ids"]
                eos = output[-1] in (self.tokenizer.eos_token_id, self.tokenizer.convert_tokens_to_ids("<|endoftext|>"))
                restored = self.tokenizer.decode(output[:-1] if eos else output, skip_special_tokens=False)
                if ((digest(prompt), digest(ids), len(ids)) != (g["prompt_sha256"], g["prompt_tokens_sha256"], g["input_tokens"])
                        or restored != g["raw"] or (g["stop"] == "eos") != eos):
                    raise ValueError("Tokenizer-level reconstruction differs.")
        g = self.generation
        if (not 0 < len(g["output_ids"]) <= LIMIT or digest(g["raw"]) != g["raw_sha256"]
                or digest(g["rendered_prompt"]) != g["prompt_sha256"]):
            raise ValueError("Generated length or raw digest differs.")
        if g["stop"] == "token_limit" and len(g["output_ids"]) == LIMIT:
            raise IncompleteOutput
        if g["stop"] != "eos":
            raise ValueError("Unexpected end reason.")
        yield [Message(role="assistant", content=g["raw"])]


def attempt(case, arm, tracer, enforce, runtime=None, saved=None, tokenizer=None, *, transport_type=LocalGeneration):
    workflow_request, cache = workflow(case, arm, demo.TOOLS, demo.MODEL)
    args = case["pending_arguments"]
    checkpoint = {case["previous_order"]: "exchange_requested", args["order_id"]: "delivered"}
    accepted = []

    def dispatch(choice):
        return demo.sandbox(choice, case["approved"], enforce=enforce, checkpoint=checkpoint, expected_arguments=args)

    def invoke(choice):
        accepted.append(choice)
        return dispatch(choice)

    before = dispatch({})["orders"]
    tools = [LocalTool(t["function"], invoke, tracer) for t in demo.TOOLS]
    transport = transport_type(case, arm, cache, tracer, runtime, saved, tokenizer)
    agent = FnCallAgent(function_list=tools, llm=transport, system_message="")
    call = None
    try:
        with closing(agent.run(workflow_request["messages"])) as batches:
            for batch in batches:
                event = batch[-1]
                if event.get("function_call"):
                    call = event["function_call"]
                if event["role"] == "function":
                    break
    except IncompleteOutput:
        pass
    if transport.emissions != 1 or sum(t.attempts for t in tools) > 1:
        raise ValueError("Unexpected number of generations or tool callbacks.")
    receipt = next((tool.receipt for tool in tools if tool.receipt is not None), None)
    if receipt is not None:
        disposition = "executed"
    elif transport.generation["stop"] == "token_limit":
        disposition = "truncated"
    elif any(tool.rejections for tool in tools):
        disposition = "tool_validation_error"
    elif call:
        disposition = "unknown_tool"
    else:
        disposition = "text"
    target = expected(case, demo.WRITE)
    parse = {"disposition": disposition, "call": call, "validated_choice": accepted[0] if accepted else None,
             "exact_expected_call": bool(accepted and accepted[0]["label"] == target["tool"]
                                         and accepted[0]["outcome"] == target["arguments"])}
    if disposition == "truncated":
        parse["exact_expected_call"] = None
    receipt = receipt or dispatch({})
    db = {"before": before, "dispatch": receipt["dispatch"], "after": receipt["orders"],
          "expected_state": receipt["orders"] == target["orders"]}
    return {"workflow_request": workflow_request, "pending_request": transport.boundary,
            "generation": transport.generation, "parser": parse}, db


def effect(before, after):
    return "unchanged" if before == after else "restored" if after else "lost"


def check_known_calls():
    """A hand-written valid call exercises the dispatcher even when every measured reply is text."""
    with instrumentation() as tracer:
        for case in DESIGN[:2]:
            args = case["pending_arguments"]
            design = {"pending_arguments": args, "previous_order": case["previous_order"]}
            request, _ = workflow(case, "original", demo.TOOLS, demo.MODEL)
            raw = "<tool_call>\n" + json.dumps({"name": demo.WRITE, "arguments": args}) + "\n</tool_call>"
            for gate in GATES.values():
                trial = replay(raw, request["messages"], case["approved"], gate, design, tracer, case["id"])
                dispatch = "blocked" if gate and not case["approved"] else "written"
                if trial["disposition"] != "executed" or trial["receipt"]["dispatch"] != dispatch:
                    raise ValueError("Known valid call did not reach its expected SQLite outcome.")


def run(runtime, saved=None, tokenizer=None, repeat=False):
    rows = []
    with instrumentation() as tracer:
        for i, case in enumerate(DESIGN):
            arms = {}
            for name in (("original",) if repeat else ("original", "repair")):
                prior = saved["cases"][i]["arms"][name] if saved else None
                measured, ungated = attempt(case, name, tracer, False, runtime, prior, tokenizer)
                replay, gated = attempt(case, name, tracer, True, saved=measured, tokenizer=tokenizer)
                if replay != measured:
                    raise ValueError("The gate changed model inputs or the proposed tool.")
                arms[name] = {**measured, "without_gate": ungated, "with_gate": gated}
                if repeat:
                    result = arms[name]
                    expected_generation = {**prior["generation"], "decode_seconds": result["generation"]["decode_seconds"]}
                    if result != {**prior, "generation": expected_generation}:
                        raise ValueError(f"Independent repeat differs for {case['id']}.")
            row = {"id": case["id"], "expected": expected(case, demo.WRITE), "arms": arms}
            if not repeat:
                row["comparison"] = {
                    gate: effect(arms["original"][gate]["expected_state"], arms["repair"][gate]["expected_state"])
                    for gate in GATES
                }
                if any(arm["parser"]["disposition"] == "truncated" for arm in arms.values()):
                    row["comparison"] = dict.fromkeys(GATES, "unscorable")
            rows.append(row)
    return {"freeze": FROZEN, "model": demo.MODEL, "revision": REVISION, "runtime": RUNTIME,
            "parser": "Qwen-Agent 0.0.34 / FnCallAgent / Nous / BaseTool native validation", "cases": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--check", type=Path)
    parser.add_argument("--repeat-baselines", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if version("qwen-agent") != "0.0.34" or any(sha256((HERE / name).read_bytes()).hexdigest() != sha
                                             for name, sha in FROZEN.items()):
        parser.error("Framework version or registered design changed.")
    if args.check:
        if args.repeat_baselines or args.output:
            parser.error("--check accepts only an optional snapshot to verify tokenization.")
        saved = json.loads(args.check.read_text())
        tokenizer = None
        if args.snapshot:
            from transformers import AutoTokenizer
            tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
        if run(None, saved, tokenizer) != saved:
            raise ValueError("Native parser, database or saved evidence differs.")
        check_known_calls()
        print("Verified eight model boundaries, native parser results and both SQLite policies.")
        return
    if not args.snapshot or not args.output or args.output.exists():
        parser.error("Generating requires a local --snapshot and an unused --output.")
    demo.verify_checkpoint(args.snapshot)
    if any(sha256((args.snapshot / name).read_bytes()).hexdigest() != sha for name, sha in MODEL_FILES.items()):
        parser.error("Snapshot configuration differs.")
    runtime = load_runtime(args.snapshot)
    if runtime.identity != RUNTIME:
        parser.error("Use the registered four-thread CPU float32 runtime.")
    saved = json.loads(args.repeat_baselines.read_text()) if args.repeat_baselines else None
    report = run(runtime, saved, runtime.tokenizer, bool(saved))
    if saved:
        arms = [r["arms"]["original"]["generation"] for r in report["cases"]]
        report = {"results_sha256": sha256(args.repeat_baselines.read_bytes()).hexdigest(),
                  "matched_baselines": [c["id"] for c in report["cases"]],
                  "input_tokens": sum(a["input_tokens"] for a in arms),
                  "output_tokens": sum(len(a["output_ids"]) for a in arms),
                  "decode_seconds": sum(a["decode_seconds"] for a in arms)}
    with args.output.open("x") as f:
        json.dump(report, f, ensure_ascii=False, indent=2, allow_nan=False)
        f.write("\n")


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
