"""Show the annotated retail session in the agentpile terminal viewer."""
import argparse
from datetime import datetime
import json
from pathlib import Path

import agentpile


def parse(value):
    return json.loads(value) if isinstance(value, str) else value


def otlp_attribute(value):
    if isinstance(value, str):
        return {"stringValue": value}
    if isinstance(value, bool):
        return {"boolValue": value}
    if isinstance(value, int):
        return {"intValue": str(value)}
    if isinstance(value, float):
        return {"doubleValue": value}
    return {"stringValue": json.dumps(value)}


def otlp_span(span):
    result = {
        "spanId": span["span_id"],
        "traceId": span["trace_id"],
        "name": span["name"],
        "attributes": [
            {"key": key, "value": otlp_attribute(value)}
            for key, value in parse(span.get("attributes") or {}).items()
            if value is not None
        ],
    }
    if span.get("parent_span_id"):
        result["parentSpanId"] = span["parent_span_id"]
    for field, target in [("start_time", "startTimeUnixNano"), ("end_time", "endTimeUnixNano")]:
        if span.get(field):
            timestamp = datetime.fromisoformat(span[field].replace("Z", "+00:00"))
            result[target] = str(int(timestamp.timestamp() * 1_000_000_000))
    return result


def transcript(spans):
    # Use the latest request snapshot and its response as one conversation.
    messages = []
    for span in reversed(sorted(spans, key=lambda span: span.get("start_time") or "")):
        attributes = parse(span.get("attributes") or {})
        if "gen_ai.input.messages" not in attributes:
            continue
        parts = parse(attributes["gen_ai.input.messages"]) + parse(attributes.get("gen_ai.output.messages") or [])
        for message in parts:
            for part in message.get("parts") or []:
                kind = part.get("type")
                if kind == "text":
                    messages.append({"role": message["role"], "content": part["content"]})
                elif kind == "tool_call":
                    arguments = part.get("arguments", {})
                    messages.append({
                        "role": "assistant",
                        "content": None,
                        "tool_calls": [{
                            "id": part["id"],
                            "type": "function",
                            "function": {
                                "name": part["name"],
                                "arguments": arguments if isinstance(arguments, str) else json.dumps(arguments),
                            },
                        }],
                    })
                elif kind == "tool_call_response":
                    content = part.get("response", part.get("result", part.get("content")))
                    messages.append({
                        "role": "tool",
                        "tool_call_id": part["id"],
                        "content": content if isinstance(content, str) else json.dumps(content),
                    })
        break
    return messages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sample", type=Path)
    parser.add_argument("--session", default="5b9274b3192a_00ab38c6")
    parser.add_argument("--transcript", action="store_true", help="View the latest snapshot as chat messages")
    args = parser.parse_args()
    with args.sample.open(encoding="utf-8") as source:
        row = next(
            (row for row in map(json.loads, source) if row["session_id"] == args.session),
            None,
        )
    if row is None:
        raise SystemExit(f"Session {args.session} not present in the sample.")
    spans = [parse(span) for span in parse(row["spans"])]
    source = transcript(spans) if args.transcript else {
        "resourceSpans": [{"scopeSpans": [{"spans": [otlp_span(span) for span in spans]}]}]
    }
    for run in agentpile.load(source, reader=None if args.transcript else "otlp-json"):
        agentpile.show(run, deps=["tool_call_id"], plain=True, max_chars=None)


if __name__ == "__main__":
    main()
