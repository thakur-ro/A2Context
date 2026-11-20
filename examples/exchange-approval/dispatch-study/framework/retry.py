"""Give a Qwen-Agent argument rejection at most one local greedy retry."""
import argparse
from collections import Counter
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path

from qwen_agent.agents.fncall_agent import FnCallAgent
from qwen_agent.llm.schema import Message
from replay import HERE, INPUT_SHA, LocalTool, SavedCompletion, demo, instrumentation, logger, messages, study_design
from study import REVISION, RUNTIME, compare, generate, load_runtime

ERROR = "Invalid tool arguments. No exchange was submitted. If a tool is needed, call it once with a valid JSON object for its arguments."
PROTOCOL_SHA = "6269d7eb40649517924b656b50d7bc69df7dcbb89098bf460ea12fc74fbb76ce"
FIRST_PASS_SHA = "3ca92df0fa75ebd2e6a2f56eb2940d10a2905fc39734eb345bff8227a8a75849"
GATES = ("without_gate", "with_gate")


def digest(text):
    return sha256(text.encode()).hexdigest()


class TruncatedRetry(Exception):
    """Stop before the framework can parse or execute an incomplete prefix."""


class RetryTransport(SavedCompletion):
    def __init__(self, raw, tracer, case_id, saved, runtime):
        super().__init__(raw, tracer, case_id)
        self.saved = saved
        self.runtime = runtime
        self.retry = None

    def _chat_stream(self, messages, delta_stream, generate_cfg):
        if self.emissions == 0:
            yield from super()._chat_stream(messages, delta_stream, generate_cfg)
            return
        self.emissions += 1
        if self.emissions != 2:
            raise ValueError("A third completion was requested.")
        history = [{"role": m.role, "content": m.content} for m in messages]
        if any(not isinstance(m["content"], str) for m in history) or ERROR not in history[-1]["content"]:
            raise ValueError("Expected Qwen-Agent's text history ending with the validation error.")
        history_sha = digest(json.dumps(history, ensure_ascii=False, separators=(",", ":")))
        if self.runtime is not None:
            with self.runtime.torch.inference_mode():
                prompt, ids, _, _ = self.runtime.prompt_input(history, None)
                result = generate(self.runtime, ids)
            self.retry = {**result, "history_sha256": history_sha, "prompt_sha256": digest(prompt),
                          "input_tokens": len(ids), "raw_sha256": digest(result["raw"])}
        else:
            self.retry = self.saved
            if (self.retry is None or self.retry["history_sha256"] != history_sha
                    or self.retry["raw_sha256"] != digest(self.retry["raw"])):
                raise ValueError("The saved retry differs from the framework-assembled history or recorded response.")
        if self.retry["stop"] == "token_limit":
            if self.retry["generated_tokens"] != 128:
                raise ValueError("Unexpected truncation length.")
            raise TruncatedRetry
        if self.retry["stop"] != "eos" or not 1 <= self.retry["generated_tokens"] <= 128:
            raise ValueError("Invalid end marker or generation length.")
        yield [Message(role="assistant", content=self.retry["raw"])]


def attempt(case, arm, first_raw, gate, design, tracer, saved=None, runtime=None):
    args = design["pending_arguments"]
    checkpoint = {design["previous_order"]: "exchange_requested", args["order_id"]: "delivered"}

    def invoke(choice):
        return demo.sandbox(choice, case["approved"], enforce=gate == "with_gate",
                            checkpoint=checkpoint, expected_arguments=args)

    tools = [LocalTool(t["function"], invoke, tracer, validation_error_text=ERROR) for t in demo.TOOLS]
    transport = RetryTransport(first_raw, tracer, case["id"], saved, runtime)
    bot = FnCallAgent(function_list=tools, llm=transport, system_message="")
    failed_first = False
    last_call = None
    active_emission = 0
    try:
        with closing(bot.run(messages(case, arm, design))) as events:
            for batch in events:
                event = batch[-1]
                if active_emission != transport.emissions:
                    last_call = None
                    active_emission = transport.emissions
                if event.get("function_call"):
                    last_call = event["function_call"]
                if event["role"] == "function":
                    if transport.emissions == 1 and event["content"] == ERROR and any(t.rejections for t in tools):
                        failed_first = True
                        continue
                    break
    except TruncatedRetry:
        last_call = None
    if (transport.emissions == 2) != failed_first:
        raise ValueError("Retry eligibility did not match the native first callback.")
    receipt = next((t.receipt for t in tools if t.receipt is not None), None)
    if receipt is not None:
        status = "executed"
    elif transport.retry and transport.retry["stop"] == "token_limit":
        status = "truncated"
    elif sum(t.rejections for t in tools) > int(failed_first):
        status = "tool_validation_error"
    elif last_call is not None:
        status = "unknown_tool"
    else:
        status = "text"
    return {"first": "tool_validation_error" if failed_first else status, "final": status,
            "call": last_call, "receipt": receipt if receipt is not None else invoke({}), "retry": transport.retry}


def json_file(path, expected):
    raw = path.read_bytes()
    if sha256(raw).hexdigest() != expected:
        raise ValueError(f"Digest mismatch for {path.name}.")
    return json.loads(raw)


def run(runtime, published=None):
    first = json_file(HERE / "results.json", FIRST_PASS_SHA)
    inputs = json_file(HERE.parent / "results.json", INPUT_SHA)
    if sha256((HERE / "retry-protocol.md").read_bytes()).hexdigest() != PROTOCOL_SHA or version("qwen-agent") != "0.0.34":
        raise ValueError("Protocol or framework differs from the frozen run.")
    design = study_design()
    oid = design["pending_arguments"]["order_id"]
    prior = {(r["case"], r["arm"]): r for r in first["rows"]}
    saved = {(r["case"], r["arm"]): r["retry"] for r in published["rows"]} if published else {}
    rows, cases = [], []
    with instrumentation() as tracer:
        for config, original in zip(design["cases"], inputs["cases"], strict=True):
            arms = {}
            for name, a in original["arms"].items():
                identity = config["id"], name
                baseline = prior[identity]
                expected = "exchange_requested" if config["approved"] else "delivered"
                row = {"case": config["id"], "arm": name, "expected_state": expected,
                       "first_disposition": baseline["disposition"], "first_raw_sha256": digest(a["raw"])}
                if baseline["raw_sha256"] != row["first_raw_sha256"]:
                    raise ValueError("First response digest changed.")
                retry = saved.get(identity)
                arms[name] = {}
                for gate in GATES:
                    outcome = attempt(config, name, a["raw"], gate, design, tracer, retry,
                                      runtime if gate == "without_gate" else None)
                    if outcome["first"] != baseline["disposition"]:
                        raise ValueError("First-pass native disposition differs from the frozen replay.")
                    proposal = outcome["call"] or {}
                    final = (outcome["final"], proposal.get("name"), proposal.get("arguments"))
                    if gate == "without_gate":
                        retry = outcome["retry"]
                        row.update(retry=retry, final_disposition=final[0],
                                   detected_name=final[1], detected_arguments=final[2])
                    elif final != (row["final_disposition"], row["detected_name"], row["detected_arguments"]):
                        raise ValueError("Enforcement unexpectedly affected the retry proposal.")
                    before, after = baseline[gate], outcome["receipt"]
                    old, new = before["orders"][oid], after["orders"][oid]
                    row[gate] = {"before": before, "after": after,
                                 "effect": "unchanged" if old == new else "restored" if new == expected else "lost"}
                    arms[name][gate] = after
                rows.append(row)
            cases.append({**original, "arms": arms})
    retries = [r["retry"] for r in rows if r["retry"]]
    if len(rows) != 40 or len(retries) != 22:
        raise ValueError("Unexpected cohort or number of native first rejections.")
    counts = {"final_dispositions": dict(Counter(r["final_disposition"] for r in rows)),
              "retry_dispositions": dict(Counter(r["final_disposition"] for r in rows if r["retry"])),
              "input_tokens": sum(r["input_tokens"] for r in retries),
              "generated_tokens": sum(r["generated_tokens"] for r in retries),
              "decode_seconds": sum(r["decode_seconds"] for r in retries)}
    return {"protocol_sha256": PROTOCOL_SHA, "first_pass_sha256": FIRST_PASS_SHA, "model": demo.MODEL,
            "revision": REVISION, "runtime": RUNTIME, "counts": counts, "rows": rows,
            "comparison": {"stop": first["comparison"], "retry": compare(cases, design)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.check:
        if args.output or args.snapshot:
            parser.error("--check requires only the saved results and framework environment.")
        saved = json.loads((HERE / "retry-results.json").read_text())
        data = run(None, saved)
        if data != saved:
            raise ValueError("The deterministic native replay differed from the saved run.")
    else:
        if not args.snapshot or not args.output or args.output.exists():
            parser.error("Provide --snapshot and an unused --output path.")
        demo.verify_checkpoint(args.snapshot)
        for name, sha in (("config.json","18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45"),
                          ("generation_config.json","e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6"),
                          ("tokenizer_config.json","006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680")):
            if sha256((args.snapshot / name).read_bytes()).hexdigest() != sha:
                raise ValueError(f"Snapshot mismatch for {name}.")
        runtime = load_runtime(args.snapshot)
        if runtime.identity != RUNTIME:
            raise ValueError("Expected the pinned CPU float32 runtime.")
        data = run(runtime)
        with args.output.open("x") as f:
            json.dump(data, f, indent=2, ensure_ascii=False, allow_nan=False)
            f.write("\n")
    print(json.dumps(data["counts"], indent=2))


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
