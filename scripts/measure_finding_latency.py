"""Measure OTLP send to readable finding on an empty, disposable sidecar."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from statistics import median, quantiles
import time

from check_sidecar import attrs, export, get, send


CALLS_PER_BATCH = 8


def measure_batch(base, batch):
    user = f"batch-{batch}"
    spans = []
    for index in range(CALLS_PER_BATCH):
        trace_id = f"{1 + batch * CALLS_PER_BATCH + index:032x}"
        messages = [{"role": "user", "content": "Look up the record."}]
        if index:
            messages.append({"role": "assistant", "tool_calls": [{"id": "lookup", "type": "function",
                             "function": {"name": "lookup", "arguments": "{}"}}]})
        messages.append({"role": "tool", "tool_call_id": "lookup", "content": "x" * 4096})
        spans.append({"traceId": trace_id, "spanId": "1111111111111111", "name": "chat",
                      "startTimeUnixNano": str(time.time_ns()), "attributes": attrs({
                          "gen_ai.operation.name": "chat", "gen_ai.request.model": "fixture",
                          "a2context.user.id": user, "a2context.request.body": json.dumps({"messages": messages}),
                      })})
    payload = export(*spans)
    started = time.perf_counter()
    for retry in range(6):
        status, body = send(base, "/v1/traces", payload, expected=None)
        if status == 200:
            break
        if status != 503 or retry == 5:
            raise RuntimeError(f"batch {batch} returned HTTP {status}: {body!r}")
        time.sleep(0.02 * 2**retry)
    card, = get(base, "findings", user=user)["rows"]
    elapsed = 1000 * (time.perf_counter() - started)
    assert card["kind"] == "tool_reference" and card["evidence"]["trace_id"] == "otlp:" + spans[0]["traceId"]
    assert card["evidence"]["detail"]["reference"]["pointer"] == "/messages/1"
    return elapsed, retry


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("endpoint", help="Fresh sidecar started with --capture-content")
    parser.add_argument("--batches", type=int, default=128)
    parser.add_argument("--batches-per-second", type=float, default=16)
    args = parser.parse_args()
    if args.batches < 2 or args.batches_per_second <= 0:
        parser.error("specify at least two batches and a positive send rate")
    health = get(args.endpoint, "health")
    if health["coverage"]["traces"] or health["accepted_batches"] or not health["content_capture"]:
        parser.error("use an empty disposable sidecar with content capture enabled")
    with ThreadPoolExecutor(max_workers=8) as pool:
        started = time.perf_counter()
        futures = []
        for i in range(args.batches):
            time.sleep(max(0, started + i / args.batches_per_second - time.perf_counter()))
            futures.append(pool.submit(measure_batch, args.endpoint, i))
        results = [future.result() for future in futures]
    seconds = time.perf_counter() - started
    durations, retries = zip(*results)
    health = get(args.endpoint, "health")
    score = get(args.endpoint, "tool-reference-score")
    calls = CALLS_PER_BATCH * args.batches
    assert (score["clear"], score["affected"]) == (calls - args.batches, args.batches)
    assert health["coverage"]["with_request"] == calls and health["accepted_batches"] == args.batches
    print(json.dumps({"calls": calls, "batches": args.batches, "wall_seconds": round(seconds, 3),
                      "observed_calls_per_second": round(calls / seconds, 2),
                      "send_to_visible_ms": {"first": round(durations[0], 3), "median": round(median(durations), 3),
                                             "p95": round(quantiles(durations, n=20, method="inclusive")[18], 3)},
                      "queue_drops": health["queue_drops"], "retried_requests": sum(retries),
                      "last_processing_lag_ms": health["processing_lag_ms"]}, indent=2))


if __name__ == "__main__":
    main()
