"""Compare inference with its existing asynchronous exporter and with a second sidecar exporter."""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.metadata import version
import json
from pathlib import Path
import platform
import statistics
import tempfile
from threading import Thread
import time

from opentelemetry.exporter.otlp.proto.http import Compression
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor

from check_sidecar import ROOT, get, server
from probe_transformers_answer_timeline import load_runtime


PROMPT = [{"role": "user", "content": "Answer with the number only. How many days are in a week?"}]


class Sink(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.send_response(200)
        self.send_header("Content-Type", "application/x-protobuf")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_):
        pass


def tracer(endpoints):
    provider = TracerProvider(resource=Resource.create({"service.name": "a2context-latency-fixture"}))
    for endpoint in endpoints:
        exporter = OTLPSpanExporter(endpoint=endpoint + "/v1/traces", compression=Compression.Gzip, timeout=5)
        provider.add_span_processor(BatchSpanProcessor(exporter, max_queue_size=256,
                                                       max_export_batch_size=16, schedule_delay_millis=75))
    return provider, provider.get_tracer("a2context.validation")


def measure(runtime, base, sidecar, samples):
    providers = [tracer([base]), tracer([base, sidecar])]
    prompt, tokens, _, _ = runtime.prompt_input(PROMPT, None)
    request = {"messages": PROMPT, "temperature": 0, "max_tokens": 4}
    inputs = json.dumps([{"role": "user", "parts": [{"type": "text", "content": PROMPT[0]["content"]}]}])
    times = [[], []]
    try:
        with runtime.torch.inference_mode():
            for _ in range(8):
                runtime.generate(prompt, tokens, request)
            for treatment in [0, 1, 1, 0] * 3:
                provider, trace = providers[treatment]
                for _ in range(samples):
                    started = time.perf_counter_ns()
                    with trace.start_as_current_span("answer") as span:
                        span.set_attribute("gen_ai.operation.name", "chat")
                        span.set_attribute("gen_ai.request.model", "Qwen2.5-0.5B-Instruct")
                        span.set_attribute("gen_ai.input.messages", inputs)
                        answer, _ = runtime.generate(prompt, tokens, request)
                        span.set_attribute("gen_ai.output.messages", json.dumps([{"role": "assistant", "content": answer}]))
                    times[treatment].append((time.perf_counter_ns() - started) / 10**6)
                assert provider.force_flush(timeout_millis=5000)
    finally:
        for provider, _ in providers:
            provider.shutdown()
    assert get(sidecar, "health")["coverage"]["traces"] == len(times[1])
    return times


def summarize(values):
    return {
        "n": len(values),
        "median_ms": round(statistics.median(values), 3),
        "p95_ms": round(statistics.quantiles(values, n=20, method="inclusive")[18], 3),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="Local Qwen2.5-0.5B-Instruct snapshot")
    parser.add_argument("--revision", required=True)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/a2context")
    parser.add_argument("--samples-per-block", type=int, default=12)
    args = parser.parse_args()
    if not args.model.is_dir() or args.samples_per_block < 2:
        parser.error("use a local model snapshot and at least two samples per block")
    runtime = load_runtime(args.model)
    runtime.torch.set_num_threads(2)
    with tempfile.TemporaryDirectory() as directory, ThreadingHTTPServer(("127.0.0.1", 0), Sink) as sink:
        thread = Thread(target=sink.serve_forever, daemon=True)
        thread.start()
        try:
            host, port = sink.server_address
            with server(args.binary, Path(directory) / "latency.sqlite", "--capture-content") as sidecar:
                baseline, treatment = measure(runtime, f"http://{host}:{port}", sidecar, args.samples_per_block)
        finally:
            sink.shutdown()
            thread.join()
    print(json.dumps({
        "model_revision": args.revision, "platform": platform.platform(),
        "torch": runtime.torch.__version__, "transformers": version("transformers"),
        "otel_sdk": version("opentelemetry-sdk"), "baseline_async_exporter": summarize(baseline),
        "with_additional_sidecar_exporter": summarize(treatment),
        "median_delta_ms": round(statistics.median(treatment) - statistics.median(baseline), 3),
    }, indent=2))


if __name__ == "__main__":
    main()
