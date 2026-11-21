"""Import pinned Exgentic v2 Parquet shards through the sidecar OTLP endpoint."""

import argparse
from collections import Counter
from datetime import datetime, timedelta, timezone
import hashlib
import json
from pathlib import Path
import time
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

from pyarrow.parquet import ParquetFile

REVISION = "ccd4c607cb708b4bb9e043bdc63a9013df35a9bc"
CONTENT = {
    "gen_ai.input.messages",
    "gen_ai.output.messages",
    "gen_ai.system_instructions",
    "gen_ai.tool.definitions",
}
COLUMNS = ["benchmark", "session_id", "harness", "success", "spans"]
BATCH_BYTES = 1_500_000
BATCH_TRACES = 64
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def timestamp(value):
    delta = datetime.fromisoformat(value) - EPOCH
    return str((delta // timedelta(microseconds=1)) * 1_000)


def imported_trace_id(row, source):
    trace_key = f'{REVISION}:{row["benchmark"]}:{row["session_id"]}:{source["trace_id"]}:{source["span_id"]}'
    return hashlib.sha256(trace_key.encode()).hexdigest()[:32]


def convert(row, source, keep_content):
    attributes = {
        key: value
        for key, value in source["attributes"].items()
        if isinstance(value, (str, int, float)) and (keep_content or key not in CONTENT)
    }
    attributes.update({
        "gen_ai.conversation.id": row["session_id"],
        "a2context.organization.id": "exgentic-v2",
        "benchmark.harness": row["harness"],
        "benchmark.success": str(row["success"]).lower() if row["success"] is not None else "unknown",
        "benchmark.original_trace_id": source["trace_id"],
    })
    span = {
        "traceId": imported_trace_id(row, source),
        "spanId": source["span_id"],
        "name": source["name"],
        "startTimeUnixNano": timestamp(source["start_time"]),
        "attributes": [
            {"key": key, "value": {"stringValue": str(value)}}
            for key, value in attributes.items()
        ],
    }
    if source["end_time"]:
        span["endTimeUnixNano"] = timestamp(source["end_time"])
    return json.dumps(span, ensure_ascii=False, separators=(",", ":")).encode()


def deliver(base, benchmark, spans):
    # Benchmark names also become the service.name workflow filter.
    resource = {"attributes": [{"key": "service.name", "value": {"stringValue": benchmark}}]}
    prefix = b'{"resourceSpans":[{"resource":' + json.dumps(resource).encode()
    payload = prefix + b',"scopeSpans":[{"spans":[' + b",".join(spans) + b']}]}]}'
    for attempt in range(5):
        try:
            request = Request(base + "/v1/traces", payload, {"Content-Type": "application/json"})
            with urlopen(request, timeout=120) as response:
                response.read()
            return
        except HTTPError as error:
            if error.code != 503 or attempt == 4:
                # Keep response content out of the study log.
                raise RuntimeError(f"{benchmark}: OTLP HTTP {error.code}") from error
            time.sleep(0.2 * 2**attempt)


def rows_from(shards):
    for shard in shards:
        for batch in ParquetFile(shard).iter_batches(batch_size=1, columns=COLUMNS):
            yield batch.to_pylist()[0]


def run(args):
    started = time.monotonic()
    counts = Counter()
    benchmark_counts = {}
    batch = []
    batch_size = 0
    current = None

    def flush():
        nonlocal batch_size
        if batch:
            deliver(args.sidecar, current, batch)
            counts["accepted_batches"] += 1
            counts["accepted_spans"] += len(batch)
            batch.clear()
            batch_size = 0

    for row in rows_from(args.shards):
        benchmark = row["benchmark"]
        if args.benchmark and benchmark not in args.benchmark:
            continue
        if current != benchmark:
            flush()
            current = benchmark
        cohort = benchmark_counts.setdefault(benchmark, Counter())
        if args.per_workflow is not None and cohort["sessions"] >= args.per_workflow:
            continue
        for source in row["spans"]:
            span = convert(row, source, args.content)
            if len(span) > BATCH_BYTES:
                span = convert(row, source, False)
                counts["oversize_metadata_only"] += 1
                cohort["oversize_metadata_only"] += 1
            if batch_size + len(span) + 1 > BATCH_BYTES or len(batch) >= BATCH_TRACES:
                flush()
            batch.append(span)
            batch_size += len(span) + 1
            cohort["spans"] += 1
        cohort["sessions"] += 1
        counts["sessions"] += 1
        if counts["sessions"] % 250 == 0:
            print(f'{counts["sessions"]} sessions read, {counts["accepted_spans"]} spans accepted', flush=True)
        if args.limit is not None and counts["sessions"] >= args.limit:
            break
    flush()
    return {
        "revision": REVISION,
        "shards": [path.name for path in args.shards],
        "content_requested": args.content,
        "counts": dict(counts),
        "by_workflow": benchmark_counts,
        "seconds": round(time.monotonic() - started, 2),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", required=True)
    parser.add_argument("--content", action="store_true")
    parser.add_argument("--benchmark", action="append")
    parser.add_argument("--limit", type=int)
    parser.add_argument("--per-workflow", type=int)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("shards", type=Path, nargs="+")
    args = parser.parse_args()
    location = urlsplit(args.sidecar)
    if location.scheme != "http" or location.hostname not in {"127.0.0.1", "localhost"} or location.path not in {"", "/"}:
        parser.error("--sidecar must be a loopback HTTP origin")
    if any(value is not None and value < 1 for value in (args.limit, args.per_workflow)):
        parser.error("--limit and --per-workflow must be positive")
    args.sidecar = args.sidecar.rstrip("/")
    args.shards.sort()
    summary = json.dumps(run(args), indent=2) + "\n"
    args.report.write_text(summary, encoding="utf-8")
    print(summary, end="")


if __name__ == "__main__":
    main()
