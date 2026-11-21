"""Export one synthetic exchange request using a standard OTLP/HTTP batch exporter."""

import argparse
import json

from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor


def approval_inputs(cached_status, customer_status):
    """Attach the two structured approval states as the request builder renders them."""
    messages = []
    declarations = []
    for source, role, value in [
        ("confirmation-cache", "system", cached_status),
        ("latest-customer-choice", "user", customer_status),
    ]:
        index = len(messages)
        messages.append({"role": role, "parts": [{"type": "text", "content": f"Exchange status: {value}"}]})
        declarations.append({
            "source_id": source,
            "source_version": "v1",
            "anchor": {"field": "gen_ai.input.messages", "message_index": index, "part_index": 0},
            "rule": {"key": "exchange_authorization", "scope": "pending order", "value": value},
        })
    return {
        "gen_ai.input.messages": json.dumps(messages),
        "a2context.context.semantics": json.dumps({"version": 1, "context": declarations}),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("conversation", help="Conversation ID to display in Threads")
    args = parser.parse_args()
    provider = TracerProvider(resource=Resource.create({"service.name": "exchange-staging"}))
    provider.add_span_processor(BatchSpanProcessor(OTLPSpanExporter()))
    try:
        tracer = provider.get_tracer("exchange-staging")
        with tracer.start_as_current_span("prepare exchange request") as span:
            span.set_attributes({
                "gen_ai.operation.name": "chat",
                "gen_ai.request.model": "synthetic",
                "gen_ai.conversation.id": args.conversation,
                **approval_inputs(cached_status="approved", customer_status="hold"),
            })
    finally:
        provider.shutdown()


if __name__ == "__main__":
    main()
