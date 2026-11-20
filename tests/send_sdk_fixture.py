"""Emit synthetic fixture spans through the OpenTelemetry SDK and its OTLP exporter."""

import json
import sys

from opentelemetry.context import Context
from opentelemetry.exporter.otlp.proto.http import Compression
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.sdk.trace.id_generator import IdGenerator
from opentelemetry.trace import NonRecordingSpan, SpanContext, TraceFlags, set_span_in_context


class FixtureIds(IdGenerator):
    """Preserve fixture locators so the same replay can reference the SDK output."""

    def generate_span_id(self):
        return int(self.record["spanId"], 16)

    def generate_trace_id(self):
        return int(self.record["traceId"], 16)


def attributes(records):
    return {item["key"]: item["value"]["stringValue"] for item in records}


def emit(document, endpoint):
    for batch in document["resourceSpans"]:
        resource = Resource.create(attributes(batch.get("resource", {}).get("attributes", [])))
        ids = FixtureIds()
        provider = TracerProvider(resource=resource, id_generator=ids)
        exporter = OTLPSpanExporter(endpoint=endpoint, compression=Compression.Gzip, timeout=5)
        provider.add_span_processor(BatchSpanProcessor(exporter, max_export_batch_size=32))
        try:
            for scope in batch["scopeSpans"]:
                tracer = provider.get_tracer("nexusweave.fixture")
                for record in scope["spans"]:
                    ids.record = record
                    context = Context()
                    if parent := record.get("parentSpanId"):
                        recorded_parent = SpanContext(ids.generate_trace_id(), int(parent, 16), False, TraceFlags(1))
                        context = set_span_in_context(NonRecordingSpan(recorded_parent), context)
                    start = int(record["startTimeUnixNano"])
                    span = tracer.start_span(record["name"], context=context, start_time=start,
                                             attributes=attributes(record["attributes"]))
                    span.end(end_time=int(record.get("endTimeUnixNano", start + 1)))
            assert provider.force_flush(timeout_millis=5000)
        finally:
            provider.shutdown()


if __name__ == "__main__":
    emit(json.load(sys.stdin), sys.argv[1])
