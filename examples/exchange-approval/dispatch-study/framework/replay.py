"""Pass saved assistant strings through Qwen-Agent and a registered local SQLite tool."""
import argparse
from collections import Counter
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path
import sys
from time import perf_counter

from jsonschema.exceptions import ValidationError
from qwen_agent.agents.fncall_agent import FnCallAgent
from qwen_agent.log import logger
from qwen_agent.llm.function_calling import BaseFnCallModel
from qwen_agent.llm.schema import Message
from qwen_agent.tools.base import BaseTool
from opentelemetry.trace import StatusCode
from telemetry import instrumentation

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from study import compare, demo, messages, study_design

INPUT_SHA = "ef86f81243df28791b64d38af1e5f39461738b5cb90c017f3f68203b0269599f"


class SavedCompletion(BaseFnCallModel):
    """Replace only the model transport. The inherited chat method handles function calling."""
    def __init__(self, raw, tracer, case_id):
        super().__init__({"model": "qwen2.5-replay", "generate_cfg": {
            "fncall_prompt_type": "nous", "use_raw_api": False, "max_input_tokens": -1}})
        self.raw = raw
        self.tracer = tracer
        self.case_id = case_id
        self.emissions = 0

    def _chat_stream(self, messages, delta_stream, generate_cfg):
        self.emissions += 1
        if self.emissions != 1:
            raise RuntimeError("Only one saved completion is available.")
        with self.tracer.start_as_current_span("Replay saved assistant completion", attributes={
                "gen_ai.operation.name": "chat", "gen_ai.request.model": "Qwen/Qwen2.5-0.5B-Instruct",
                "gen_ai.conversation.id": self.case_id, "exchange.transport": "saved_text"}):
            completion = Message(role="assistant", content=self.raw)
        yield [completion]

    def _chat_no_stream(self, messages, generate_cfg):
        raise RuntimeError("This replay supplies one full, non-delta chunk.")


class LocalTool(BaseTool):
    def __init__(self, schema, invoke, tracer, *, validation_error_text=None):
        self.name, self.description = schema["name"], schema["description"]
        # Qwen-Agent accepts this declaration subset. SQLite still checks all four exact IDs.
        self.parameters = {k: schema["parameters"][k] for k in ("type", "properties", "required")}
        self.invoke = invoke
        self.tracer = tracer
        self.receipt = None
        self.attempts = 0
        self.rejections = 0
        self.validation_error_text = validation_error_text
        super().__init__()

    def call(self, params, **kwargs):
        self.attempts += 1
        with self.tracer.start_as_current_span("Run registered tool", record_exception=False,
                set_status_on_exception=False, attributes={"gen_ai.operation.name": "execute_tool",
                                                          "gen_ai.tool.name": self.name}) as span:
            try:
                arguments = self._verify_json_format_args(params)
            except (ValueError, ValidationError):
                self.rejections += 1
                span.set_attribute("exchange.result", "argument_validation_error")
                span.set_status(StatusCode.ERROR, "Argument validation failed")
                if self.validation_error_text is not None:
                    return self.validation_error_text
                raise
            with self.tracer.start_as_current_span("Dispatch to SQLite") as child:
                self.receipt = self.invoke({"kind": "tool_call", "label": self.name, "outcome": arguments})
                child.set_attribute("exchange.result", self.receipt["dispatch"])
            span.set_attribute("exchange.result", self.receipt["dispatch"])
        return self.receipt


def replay(raw, request, approved, enforce, design, tracer, case_id):
    args = design["pending_arguments"]
    checkpoint = {design["previous_order"]: "exchange_requested", args["order_id"]: "delivered"}

    def invoke(choice):
        return demo.sandbox(choice, approved, enforce=enforce, checkpoint=checkpoint, expected_arguments=args)

    tools = [LocalTool(tool["function"], invoke, tracer) for tool in demo.TOOLS]
    transport = SavedCompletion(raw, tracer, case_id)
    bot = FnCallAgent(function_list=tools, llm=transport, system_message="")
    call = None
    response = None
    with closing(bot.run(request)) as stream:
        for events in stream:
            for event in events:
                if event.get("function_call"):
                    call = event["function_call"]
                if event["role"] == "function":
                    response = event["content"]
            if response is not None:
                break
    if transport.emissions != 1 or sum(t.attempts for t in tools) > 1:
        raise ValueError("Expected one assistant completion and at most one tool callback.")
    receipt = next((tool.receipt for tool in tools if tool.receipt is not None), None)
    if receipt is not None:
        disposition = "executed"
    elif any(t.attempts for t in tools):
        if response is None or "An error occurred when calling tool" not in response:
            raise ValueError("Missing Qwen-Agent validation error response.")
        disposition = "tool_validation_error"
    elif call:
        disposition = "unknown_tool"
    else:
        disposition = "text"
    return {"disposition": disposition, "detected_name": call["name"] if call else None,
            "detected_arguments": call["arguments"] if call else None,
            "receipt": receipt if receipt is not None else invoke({})}


def run(tracer):
    package = version("qwen-agent")
    if package != "0.0.34":
        raise ValueError("This replay was registered against Qwen-Agent 0.0.34.")
    source = (HERE.parent / "results.json").read_bytes()
    if sha256(source).hexdigest() != INPUT_SHA:
        raise ValueError("Saved raw generations changed.")
    inputs, design = json.loads(source), study_design()
    target = design["pending_arguments"]["order_id"]
    counts, rows = Counter(), []
    cases = []
    for case, original in zip(design["cases"], inputs["cases"], strict=True):
        arms = {}
        for name, arm in original["arms"].items():
            if arm["stop"] != "eos":
                raise ValueError("This replay expects a completed generation.")
            result = {"case": case["id"], "arm": name, "raw_sha256": sha256(arm["raw"].encode()).hexdigest()}
            for key, enforce in (("without_gate", False), ("with_gate", True)):
                attrs = {"gen_ai.operation.name": "invoke_agent", "gen_ai.conversation.id": case["id"],
                         "exchange.arm": name, "exchange.policy": key}
                with tracer.start_as_current_span("Run Qwen-Agent", attributes=attrs) as span:
                    trial = replay(arm["raw"], messages(case, name, design), case["approved"], enforce,
                                   design, tracer, case["id"])
                    span.set_attribute("exchange.disposition", trial["disposition"])
                    span.set_attribute("exchange.result", trial["receipt"]["dispatch"])
                disposition = trial.pop("disposition")
                receipt = trial.pop("receipt")
                if not enforce:
                    result.update(disposition=disposition, **trial)
                    counts[disposition] += 1
                elif (disposition, trial["detected_name"], trial["detected_arguments"]) != (
                        result["disposition"], result["detected_name"], result["detected_arguments"]):
                    raise ValueError("Enforcement unexpectedly affected parsing.")
                result[key] = receipt
                counts[key + "_state_differences"] += arm[key]["orders"][target] != receipt["orders"][target]
            rows.append(result)
            arms[name] = result
        cases.append({**original, "arms": arms})
    return {"qwen_agent": package, "source_sha256": INPUT_SHA, "counts": dict(counts),
            "rows": rows, "comparison": compare(cases, design)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--sidecar-port", type=int, help="Export batched OTLP/HTTP to this loopback port.")
    args = parser.parse_args()
    if bool(args.output) == args.check or (args.output and args.output.exists()):
        parser.error("Use --check or give a new --output path.")
    start = perf_counter()
    with instrumentation(args.sidecar_port) as tracer:
        data = run(tracer)
    elapsed = perf_counter() - start
    if args.check:
        if data != json.loads((HERE / "results.json").read_text()):
            raise ValueError("The framework replay differs from the saved receipts.")
    else:
        with args.output.open("x") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
    print(json.dumps(data["counts"]), f"{elapsed:.3f} seconds")


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
