"""Use Qwen-Agent's native Nous parser/validator and the hash-pinned in-memory TravelAPI."""
from collections import Counter
from contextlib import closing
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import sys
from time import perf_counter

from inputs import HERE, digest, new_sim, public_state

ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "examples/exchange-approval/dispatch-study/framework"))
from replay import BaseTool, FnCallAgent, Message, SavedCompletion, ValidationError
from study import LIMIT, generate, load_runtime
from score_qwen_prefill import first_step

SYSTEM = "You are a travel agent. Use the available travel tools to complete the customer's request and report what happened."
MARKER = "[source unavailable]"
SOURCES = ("system", "tools", "customer")


def text_hash(text):
    return sha256(text.encode()).hexdigest()


class Boundary(Exception):
    pass


class Transport(SavedCompletion):
    def __init__(self, runtime, tracer, tid, audit, saved, tokenizer, probe):
        super().__init__(None, tracer, tid)
        self.runtime, self.audit, self.saved = runtime, audit, saved
        self.tokenizer, self.probe = tokenizer, probe
        self.receipts = []
        self.distribution = None

    def _chat_stream(self, messages, delta_stream, generate_cfg):
        if len(self.receipts) == 3:
            raise Boundary("completion_bound")
        history = [{"role": m.role, "content": m.content} for m in messages]
        if any(not isinstance(m["content"], str) for m in history):
            raise ValueError("Unexpected non-text framework history")
        receipt = {"history_sha256": digest(history)}
        ids = None
        if self.tokenizer:
            prompt = self.tokenizer.apply_chat_template(history, tools=None, tokenize=False, add_generation_prompt=True)
            ids = self.tokenizer.encode(prompt, add_special_tokens=False)
            receipt.update(prompt_sha256=text_hash(prompt), input_tokens=len(ids))
        if self.probe:
            start = perf_counter()
            with self.runtime.torch.inference_mode():
                self.distribution = first_step(self.runtime, ids)
            receipt["probe_seconds"] = perf_counter() - start
            self.receipts.append(receipt)
            raise Boundary("probe")
        if self.runtime:
            with self.runtime.torch.inference_mode():
                generation = generate(self.runtime, ids)
            raw = generation.pop("raw")
            receipt.update(generation)
            self.audit.append({"raw": raw, "stop": receipt["stop"], "generated_tokens": receipt["generated_tokens"]})
        else:
            saved = self.saved[len(self.receipts)]
            raw = saved["raw"]
            receipt.update(stop=saved["stop"], generated_tokens=saved.get("generated_tokens"))
        receipt["raw_sha256"] = text_hash(raw)
        self.receipts.append(receipt)
        if receipt["stop"] == "token_limit":
            raise Boundary("truncated")
        if receipt["stop"] != "eos":
            raise ValueError("Unexpected stopping reason")
        yield [Message(role="assistant", content=raw)]


class TravelTool(BaseTool):
    def __init__(self, schema, sim, events, observations=None):
        self.name, self.description, self.parameters = schema["name"], schema["description"], schema["parameters"]
        self.sim, self.events = sim, events
        self.observations = observations
        super().__init__()

    def call(self, params, **kwargs):
        event = self.events[-1]
        observation = {"name": self.name}
        if self.observations is not None:
            self.observations.append(observation)
        try:
            args = self._verify_json_format_args(params)
        except (ValueError, ValidationError, TypeError):
            event["status"] = "schema_rejection"
            return "The travel tool rejected these arguments."
        event["arguments_sha256"] = digest(args)
        observation["arguments"] = args
        try:
            response = getattr(self.sim, self.name)(**args)
        except (TypeError, KeyError, ValueError, IndexError) as exc:
            event["status"] = "simulator_exception"
            observation["simulator_exception_type"] = type(exc).__name__
            return f"The travel simulator returned {type(exc).__name__}."
        event.update(status="executed", response_sha256=digest(response))
        observation["returned_domain_error"] = isinstance(response, dict) and "error" in response
        return response


class TravelAgent(FnCallAgent):
    def __init__(self, events, **kwargs):
        self.events = events
        super().__init__(**kwargs)

    def _call_tool(self, tool_name, tool_args, **kwargs):
        if len(self.events) == 6:
            raise Boundary("callback_bound")
        self.events.append({"name": tool_name, "native_arguments_sha256": digest(tool_args), "status": "unknown_tool"})
        return super()._call_tool(tool_name, tool_args, **kwargs)


def contains_ordered(actual, required):
    remaining = iter(actual)
    return all(any(candidate == expected for candidate in remaining) for expected in required)


def trial(pkt, schemas, cls, tracer, *, runtime=None, tokenizer=None, saved=None, source=None, probe=False,
          observations=None, system_message=SYSTEM):
    sim = new_sim(cls, pkt["state"])
    events, audit = [], []
    schema_list = deepcopy(schemas)
    if source == "tools":
        for item in schema_list:
            item["description"] = MARKER
    messages = deepcopy(pkt["messages"])
    if source == "customer":
        messages[0]["content"] = MARKER
    transport = Transport(runtime, tracer, pkt["id"], audit, saved, tokenizer, probe)
    bot = TravelAgent(events, function_list=[TravelTool(s, sim, events, observations) for s in schema_list],
                      llm=transport, system_message=MARKER if source == "system" else system_message)
    stop = "normal"
    try:
        with closing(bot.run(messages)) as stream:
            for _ in stream:
                pass
    except Boundary as limit:
        stop = str(limit)
    if probe:
        if stop != "probe":
            raise ValueError("Probe did not reach the model boundary.")
        return transport.distribution, transport.receipts[0]
    if saved is not None and len(saved) != len(transport.receipts):
        raise ValueError("The native control/replay consumed a different number of outputs.")
    row = workflow_score(pkt, cls, sim, events, stop)
    return {**row, "generations": transport.receipts}, audit


def workflow_score(pkt, cls, sim, events, stop):
    valid = [e for e in events if e["status"] == "executed"]
    reference = new_sim(cls, pkt["state"])
    outputs = [getattr(reference, n)(**a) for n, a in pkt["ref"]]
    actions_ok = contains_ordered([(e["name"], e["arguments_sha256"]) for e in valid],
                                  [(n, digest(a)) for n, a in pkt["ref"]])
    results_ok = not (Counter(map(digest, outputs)) - Counter(e["response_sha256"] for e in valid))
    state_ok = sim == reference
    return {"id": pkt["id"], "stop": stop, "calls": events,
            "state_sha256": digest(public_state(sim)), "state_match": state_ok, "required_actions": actions_ok,
            "required_responses": results_ok, "usable": stop == "normal" and state_ok and actions_ok and results_ok}


def control(pkt, schemas, cls, tracer):
    known = [{"raw": "<tool_call>\n" + json.dumps({"name": name, "arguments": args}) + "\n</tool_call>", "stop": "eos"}
             for name, args in pkt["ref"]]
    text = [{"raw": "No tool invoked.", "stop": "eos"}]
    positive, _ = trial(pkt, schemas, cls, tracer, saved=known + text)
    negative, _ = trial(pkt, schemas, cls, tracer, saved=text)
    if not positive["usable"] or negative["usable"] or negative["required_actions"] or negative["required_responses"]:
        raise ValueError("Reference or no-call negative controls failed.")
    return {"id": pkt["id"], "known_valid": positive["usable"], "reference_final_state_sha256": positive["state_sha256"],
            "known_valid_callbacks": len(positive["calls"]), "no_call_state_match": negative["state_match"],
            "no_call_required_actions": negative["required_actions"], "no_call_required_responses": negative["required_responses"]}
