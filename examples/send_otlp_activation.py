"""Send a saved Qwen inference and its measured activation report to NexusWeave."""

import argparse
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor


def rekey_sources(report, ids):
    for source in report["sources"]:
        source["context_id"] = ids[source["context_id"]]
    for step in report["checkpoints"]:
        checks = step["checks"]
        checks["source_context_id"] = ids[checks["source_context_id"]]
        step["edited_margins"] = {ids[key]: value for key, value in step["edited_margins"].items()}
        for sweep in step["sweeps"]:
            sweep["source_context_id"] = ids[sweep["source_context_id"]]
    output = report["output_scores"]
    checks = output["checks"]
    checks["source_context_id"] = ids[checks["source_context_id"]]
    output["sources"] = {ids[key]: value for key, value in output["sources"].items()}
    for branch in report["generation_branches"]:
        branch["source_context_id"] = ids[branch["source_context_id"]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040", help="Inspector base URL.")
    parser.add_argument("--conversation-id", default="qwen-measurement-demo")
    args = parser.parse_args()
    base_url = args.url.rstrip("/")
    fixtures = Path(__file__).parent / "runtime-comparison"
    report = json.loads((fixtures / "generation-branches-transformers.json").read_text(encoding="utf-8"))
    runs = json.loads((fixtures / "evidence.json").read_text(encoding="utf-8"))
    run = next(run for run in runs if run["id"] == report["location"]["run_id"])
    call = next(call for call in run["calls"] if call["id"] == report["location"]["call_id"])
    output = {"field": "gen_ai.output.messages", "message_index": 0, "part_index": 0}
    semantics = {
        "version": 1,
        "request_sources": [
            {"id": item["id"], "source_id": item["id"], "kind": item["kind"],
             "role": item["role"], "request_fragment": item["request_fragment"]}
            for item in call["context"]
        ],
        "relationships": [
            {"context": {"request_source_id": item["id"]}, "decision": output,
             "relation": "included in the model request"}
            for item in call["context"]
        ],
    }
    provider = TracerProvider()
    exporter = OTLPSpanExporter(endpoint=base_url + "/v1/traces")
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    tracer = provider.get_tracer("nexusweave-activation-example")
    try:
        with tracer.start_as_current_span("Return and refund answer", attributes={
            "gen_ai.operation.name": "chat",
            "gen_ai.request.model": report["model_id"],
            "gen_ai.conversation.id": args.conversation_id,
            "nexusweave.organization.id": "qwen-demo-org",
            "nexusweave.user.id": "qwen-demo-user",
            "nexusweave.turn.id": "returns-and-refunds",
            "nexusweave.request.body": call["recorded_request"]["body"],
            "nexusweave.context.semantics": json.dumps(semantics),
            "gen_ai.output.messages": json.dumps([{
                "role": "assistant", "parts": [{"type": "text", "content": report["baseline_answer"]}],
            }]),
        }) as span:
            context = span.get_span_context()
    finally:
        provider.shutdown()

    run_id = f"otlp:{context.trace_id:032x}"
    call_id = f"{context.span_id:016x}"
    decision_id = "output:0:0"
    report["location"] = {"run_id": run_id, "call_id": call_id, "decision_id": decision_id}
    rekey_sources(report, {item["id"]: f"request-source:{item['id']}" for item in call["context"]})
    request = Request(
        base_url + "/api/activation-reports",
        json.dumps(report).encode("utf-8"),
        {"Content-Type": "application/json"},
    )
    query = urlencode({
        "v": 1, "view": "conversation", "run": run_id, "call": call_id, "decision": decision_id,
    })
    with urlopen(request, timeout=10):
        print(f"{base_url}/#{query}")


if __name__ == "__main__":
    main()
