"""Run the pinned LFM chat template and tool parser against the isolated BFCL TravelAPI."""
from copy import deepcopy
from hashlib import file_digest
from importlib.metadata import version
import json
from pathlib import Path
import sys
from time import perf_counter

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from inputs import digest, new_sim
from staging import MARKER, SYSTEM, TravelTool, text_hash, workflow_score
from mlx_activation import prompt_input
from activation_timeline import tool_payload


def load_model(snapshot):
    import mlx.core as mx
    from mlx_lm import load

    if version("mlx-lm") != "0.32.2" or version("mlx") != "0.32.4" or version("qwen-agent") != "0.0.34":
        raise ValueError("Use the pinned MLX and argument-validator environment.")
    for name, expected in json.loads((HERE / "files.json").read_text()).items():
        with (snapshot / name).open("rb") as stream:
            if file_digest(stream, "sha256").hexdigest() != expected:
                raise ValueError("Unexpected checkpoint file: " + name)
    if mx.default_device().type != mx.gpu:
        raise ValueError("The frozen calibration uses the Metal GPU.")
    return load(str(snapshot))


def inputs(pkt, schemas, edit=None):
    tools = schemas
    if edit == "tools":
        tools = [{**schema, "description":MARKER} for schema in schemas]
    messages = [{"role":"system","content":MARKER if edit == "system" else SYSTEM}, *deepcopy(pkt["messages"])]
    if edit == "customer":
        messages[-1]["content"] = MARKER
    return messages, tools


def proposals(tokenizer, raw, schemas):
    payload, _ = tool_payload(raw, tokenizer.tool_call_start, tokenizer.tool_call_end)
    if payload is None:
        return []
    parsed = tokenizer.tool_parser(payload, schemas)
    return parsed if isinstance(parsed, list) else [parsed]


def trial(model, tokenizer, pkt, schemas, cls, *, edit=None, scripted=None):
    from mlx_lm import stream_generate
    from mlx_lm.sample_utils import make_sampler

    sim = new_sim(cls, pkt["state"])
    events, receipts, dispositions = [], [], []
    messages, tools = inputs(pkt, schemas, edit)
    registry = {s["name"]:TravelTool(s, sim, events, dispositions) for s in schemas}
    stop = "completion_bound"
    for index in range(3):
        prompt, ids, _, _ = prompt_input(tokenizer, messages, tools)
        if scripted is None:
            start = perf_counter()
            output = list(stream_generate(model, tokenizer, ids, max_tokens=128, sampler=make_sampler(temp=0)))
            raw = "".join(event.text for event in output)
            receipt = {"prompt_sha256":text_hash(prompt), "history_sha256":digest(messages), "raw_sha256":text_hash(raw),
                       "input_tokens":len(ids), "generated_tokens":output[-1].generation_tokens,
                       "stop":output[-1].finish_reason, "decode_seconds":perf_counter()-start}
            receipts.append(receipt)
            if receipt["stop"] != "stop":
                stop = "truncated"
                break
        else:
            raw = scripted[index]
        try:
            calls = proposals(tokenizer, raw, tools)
        except (ValueError, SyntaxError, KeyError, TypeError):
            stop = "parser_error"
            break
        if not calls:
            stop = "normal"
            break
        if len(events) + len(calls) > 6:
            stop = "callback_bound"
            break
        results = []
        for call in calls:
            name, args = call["name"], call["arguments"]
            event = {"name":name,"native_arguments_sha256":digest(args),"status":"unknown_tool"}
            events.append(event)
            result = registry[name].call(json.dumps(args)) if name in registry else "Unknown travel tool."
            results.append(result)
        messages.extend([{"role":"assistant", "content":raw},
                         {"role":"tool", "content":json.dumps(results, ensure_ascii=False)}])
    return {**workflow_score(pkt, cls, sim, events, stop), "generations":receipts,
            "domain_errors":sum(bool(d.get("returned_domain_error")) for d in dispositions)}


def controls(tokenizer, pkt, schemas, cls):
    def syntax(name, args):
        return name + "(" + ", ".join(f"{k}={v!r}" for k,v in args.items()) + ")"
    raw = tokenizer.tool_call_start + "[" + ", ".join(syntax(n,a) for n,a in pkt["ref"]) + "]" + tokenizer.tool_call_end
    positive = trial(None, tokenizer, pkt, schemas, cls, scripted=[raw, "Done."])
    negative = trial(None, tokenizer, pkt, schemas, cls, scripted=["No call."])
    if not positive["usable"] or negative["required_actions"] or negative["required_responses"]:
        raise ValueError("Reference or no-call control failed on " + pkt["id"])
    return {"id":pkt["id"],"positive":positive,"no_call":negative}
