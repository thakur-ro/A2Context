"""Reproduce the synthetic Qwen2 routing cohort and its source masks."""

import argparse
from copy import deepcopy
import json
from pathlib import Path

from probe_source_visibility import measure, output_targets
from probe_transformers_answer_timeline import load_runtime

MODEL = "Qwen/Qwen2.5-0.5B-Instruct"
REVISION = "eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2"
PAIRS = [("request-type", "product-id"), ("tool-stock", "tool-price"),
         ("routing-instruction", "request-type"), ("routing-instruction", "tool-stock")]
CONTINUATIONS = [(7, "tool-stock"), (7, "tool-price"), (7, "tool-price", "tool-stock"), (17, "product-id")]


def build_run(template, case):
    run = deepcopy(template)
    run.update(id="qwen-routing-" + case["id"], **{
        key: case[key] for key in ("organization_id", "user_id", "conversation_id")
    })
    call = run["calls"][0]
    call.update(turn_id=case["turn_id"], started_at=case["started_at"])
    request = json.loads(call["recorded_request"]["body"])
    messages = request["messages"]
    tools = [item for item in call["context"] if item["kind"] == "tool_definition"]
    messages[1]["content"] = f'Request type: {case["tool"]}.'
    messages[2]["content"] = f'Product ID: {case["product_id"]}.'
    message_ids = [item["id"] for item in call["context"] if item["kind"] == "message"]
    history = case.get("history", [])
    message_ids[1:1] = [item["id"] for item in history]
    messages[1:1] = [{"role": item["role"], "content": item["content"]} for item in history]
    call["context"] = [
        {
            "id": name,
            "kind": "message",
            "role": message["role"],
            "text": message["content"],
            "availability": "included",
            "source_id": name,
            "source_version": "v1",
            "evidence": {"field": "request.messages", "message_index": index},
            "request_fragment": {
                "pointer": f"/messages/{index}/content",
                "text_range": {"start_byte": 0, "end_byte": len(message["content"].encode())},
            },
        }
        for index, (name, message) in enumerate(zip(message_ids, messages, strict=True))
    ] + tools
    call["recorded_request"]["body"] = json.dumps(request, ensure_ascii=False, separators=(",", ":"))
    call["decisions"][0].update(label=case["tool"], outcome={"product_id": case["product_id"]})
    return run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="Local weights for the pinned Qwen2.5 checkpoint")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / "examples/runtime-comparison"
    cases = json.loads((root / "visibility-routing-cohort/cohort.json").read_text())
    (template,) = json.loads((root / "visibility-routing/evidence.json").read_text())
    runtime = load_runtime(args.model)
    runs, reports = [], []
    with runtime.torch.inference_mode():
        for case in cases:
            run = build_run(template, case)
            call = run["calls"][0]
            decision = call["decisions"][0]
            location = {"run_id": run["id"], "call_id": call["id"], "decision_id": decision["id"]}
            report = measure(runtime, args.model, MODEL, REVISION, location, call, decision,
                             pairs=PAIRS, continuations=CONTINUATIONS)
            targets = [("routing.tool", 7, 8, case["tool"]), ("routing.product_id", 17, 19, str(case["product_id"]))]
            for key, start, end, expected in targets:
                if "".join(report["fragments"][start:end]) != expected:
                    raise ValueError(f"{run['id']} needs a verified token span for {key}.")
            report["targets"] = output_targets(
                [(key, start, end) for key, start, end, _ in targets], len(report["output_token_ids"]),
            )
            runs.append(run)
            reports.append(report)
    args.output.mkdir(parents=True, exist_ok=True)
    for name, value in [("evidence.json", runs), ("reports.json", reports)]:
        (args.output / name).write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
