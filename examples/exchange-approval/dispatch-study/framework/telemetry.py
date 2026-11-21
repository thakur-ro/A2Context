"""Application-owned instrumentation for the offline Qwen-Agent replay."""
from contextlib import contextmanager

from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider


@contextmanager
def instrumentation(port=None):
    provider = TracerProvider(resource=Resource({"service.name": "qwen-agent-exchange-replay"}))
    try:
        if port is not None:
            if not 1 <= port <= 65535:
                raise ValueError("Expected a loopback sidecar port from 1 to 65535.")
            from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
            from opentelemetry.sdk.trace.export import BatchSpanProcessor
            import requests

            session = requests.Session()
            session.trust_env = False
            exporter = OTLPSpanExporter(endpoint=f"http://127.0.0.1:{port}/v1/traces", session=session, timeout=5)
            provider.add_span_processor(BatchSpanProcessor(exporter, max_export_batch_size=16, max_queue_size=512))
        yield provider.get_tracer("a2context.qwen_agent_replay")
        if not provider.force_flush(timeout_millis=10000):
            raise RuntimeError("The span processor did not finish exporting.")
    finally:
        provider.shutdown()
