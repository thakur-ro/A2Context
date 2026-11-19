"""Send a saved Qwen inference and its controlled replay to a live inspector."""

import argparse
import json
import time
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import uuid4


def prepare(conversation_id):
    fixtures = Path(__file__).parent / "context-screening" / "replay"
    report = json.loads((fixtures / "report.json").read_text(encoding="utf-8"))
    run = json.loads((fixtures / "evidence.json").read_text(encoding="utf-8"))[0]
    call = run["calls"][0]
    body = call["recorded_request"]["body"]
    request = json.loads(body)
    inputs = [
        {"role": message["role"], "parts": [{"type": "text", "content": message["content"]}]}
        for message in request["messages"]
    ]
    context_ids = {}
    declarations = []
    for item in call["context"]:
        index = item["evidence"]["message_index"]
        context_ids[item["id"]] = f"input:{index}:0"
        if item["source_id"]:
            declarations.append({
                "anchor": {"field": "gen_ai.input.messages", "message_index": index, "part_index": 0},
                "source_id": item["source_id"],
                "source_version": item["source_version"],
            })
    trace_id, span_id = uuid4().hex, uuid4().hex[:16]
    attributes = {
        "gen_ai.operation.name": "chat",
        "gen_ai.request.model": request["model"],
        "gen_ai.conversation.id": conversation_id,
        "nexusweave.organization.id": run["organization_id"],
        "nexusweave.user.id": run["user_id"],
        "nexusweave.turn.id": call["turn_id"],
        "nexusweave.request.body": body,
        "nexusweave.context.semantics": json.dumps({"version": 1, "context": declarations}),
        "gen_ai.input.messages": json.dumps(inputs),
        "gen_ai.output.messages": json.dumps([{
            "role": "assistant",
            "parts": [{"type": "text", "content": report["recorded"]["outcome"]}],
        }]),
    }
    timestamp = time.time_ns()
    span = {
        "traceId": trace_id,
        "spanId": span_id,
        "name": "Journal return answer",
        "startTimeUnixNano": str(timestamp - 1),
        "endTimeUnixNano": str(timestamp),
        "attributes": [
            {"key": key, "value": {"stringValue": value}} for key, value in attributes.items()
        ],
    }
    report["location"] = {"run_id": f"otlp:{trace_id}", "call_id": span_id, "decision_id": "output:0:0"}
    for variant in report["variants"]:
        intervention = variant["intervention"]
        intervention["context_id"] = context_ids[intervention["context_id"]]
    return {"resourceSpans": [{"scopeSpans": [{"spans": [span]}]}]}, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040", help="Inspector base URL.")
    parser.add_argument("--conversation-id", default="journal-live-replay")
    args = parser.parse_args()
    base_url = args.url.rstrip("/")
    trace, report = prepare(args.conversation_id)
    for path, payload in [("/v1/traces", trace), ("/api/replay-reports", report)]:
        request = Request(
            base_url + path, json.dumps(payload).encode(), {"Content-Type": "application/json"},
        )
        with urlopen(request, timeout=10):
            pass
    location = report["location"]
    query = urlencode({
        "v": 1, "view": "decisions", "run": location["run_id"],
        "call": location["call_id"], "decision": location["decision_id"],
        "replay": report["id"], "variant": "input:1:0",
    })
    print(f"{base_url}/#{query}")


if __name__ == "__main__":
    main()
