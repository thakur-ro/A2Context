"""Send a synthetic inventory workflow through the OpenTelemetry HTTP exporter."""

import argparse
import json

from opentelemetry.exporter.otlp.proto.http import Compression
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="http://127.0.0.1:8040/v1/traces")
    parser.add_argument("--capture-request", action="store_true", help="Include the original synthetic request body.")
    parser.add_argument("--request-sources", action="store_true", help="Capture a prompt with declared source ranges.")
    parser.add_argument("--conversation-id", help="Use a specific conversation ID for this example.")
    args = parser.parse_args()

    provider = TracerProvider(resource=Resource.create({
        "service.name": "a2context-inventory-example",
        "a2context.organization.id": "sdk-demo-org",
    }))
    exporter = OTLPSpanExporter(endpoint=args.endpoint, compression=Compression.Gzip)
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    tracer = provider.get_tracer("a2context-inventory-example")
    if args.request_sources:
        demo_thread = "sdk-inventory-prompt-sources"
    else:
        demo_thread = "sdk-inventory-captured-thread" if args.capture_request else "sdk-inventory-thread"
    scope = {
        "gen_ai.conversation.id": args.conversation_id or demo_thread,
        "a2context.user.id": "sdk-demo-user",
        "a2context.turn.id": "stock-question",
    }
    inference = {
        **scope,
        "gen_ai.operation.name": "chat",
        "gen_ai.request.model": "synthetic-inventory",
    }
    tool_call = {
        "type": "tool_call", "id": "inventory-lookup",
        "name": "look_up_inventory", "arguments": {"sku": "MUG-42"},
    }
    question = "Check SKU MUG-42."
    tools = [{"name": "look_up_inventory", "description": "Read stock by SKU."}]
    tool_anchor = {"field": "gen_ai.tool.definitions", "part_index": 0}
    decision_anchor = {"field": "gen_ai.output.messages", "message_index": 0, "part_index": 0}
    semantics = {
        "version": 1,
        "context": [{"anchor": tool_anchor, "source_id": "inventory-lookup-tool", "source_version": "v1"}],
        "relationships": [{
            "context": tool_anchor,
            "decision": decision_anchor,
            "relation": "available for tool selection",
        }],
    }
    capture = {}
    messages = [{"role": "user", "content": question}]
    if args.request_sources:
        system_prompt = "Inventory workflow ·\n"
        semantics["request_sources"] = []
        for source_id, kind, text in [
            ("inventory-routing-policy", "instruction", "Look up inventory before reporting availability."),
            ("inventory-sku-skill", "skill", "Use the exact SKU supplied by the user."),
        ]:
            start = len(system_prompt.encode("utf-8"))
            system_prompt += text
            source = {
                "id": source_id,
                "source_id": source_id,
                "source_version": "v1",
                "kind": kind,
                "role": "system",
                "request_fragment": {
                    "pointer": "/messages/0/content",
                    "text_range": {"start_byte": start, "end_byte": len(system_prompt.encode("utf-8"))},
                },
            }
            semantics["request_sources"].append(source)
            semantics["relationships"].append({
                "context": {"request_source_id": source_id},
                "decision": decision_anchor,
                "relation": "applies to inventory selection",
            })
            system_prompt += "\n"
        messages.insert(0, {"role": "system", "content": system_prompt})
    if args.capture_request or args.request_sources:
        capture["a2context.request.body"] = json.dumps({
            "model": inference["gen_ai.request.model"],
            "messages": messages,
            "tools": tools,
            "temperature": 0,
        })
    result = {"sku": "MUG-42", "quantity": 42}
    try:
        with tracer.start_as_current_span("inventory turn", attributes=scope):
            with tracer.start_as_current_span("choose inventory tool", attributes={
                **inference,
                **capture,
                "gen_ai.input.messages": json.dumps([
                    {"role": message["role"], "parts": [{"type": "text", "content": message["content"]}]}
                    for message in messages
                ]),
                "gen_ai.tool.definitions": json.dumps(tools),
                "a2context.context.semantics": json.dumps(semantics),
                "gen_ai.output.messages": json.dumps([{"role": "assistant", "parts": [tool_call]}]),
            }):
                pass
            with tracer.start_as_current_span("inventory lookup", attributes={
                **scope,
                "gen_ai.operation.name": "execute_tool",
                "gen_ai.tool.name": tool_call["name"],
                "gen_ai.tool.call.id": tool_call["id"],
                "gen_ai.tool.call.result": json.dumps(result),
            }):
                pass
            with tracer.start_as_current_span("answer stock question", attributes={
                **inference,
                "gen_ai.input.messages": json.dumps([{
                    "role": "tool",
                    "parts": [{"type": "tool_call_response", "id": tool_call["id"], "response": result}],
                }]),
                "gen_ai.output.messages": json.dumps([{
                    "role": "assistant", "parts": [{"type": "text", "content": "42 units are in stock."}],
                }]),
            }):
                pass
    finally:
        provider.shutdown()


if __name__ == "__main__":
    main()
