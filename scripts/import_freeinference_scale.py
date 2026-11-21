"""Import the FreeInference request and session Parquet tables as metadata-only OTLP."""

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import time
from urllib.parse import urlsplit

from pyarrow.parquet import ParquetFile

from import_exgentic_scale import BATCH_TRACES, deliver

REVISION = "bb26cf72c9c20f2b05f765d88548156a74d3c253"
COLUMNS = ["session_id", "step", "start_ms", "end_ms", "model", "provider",
           "prompt_tokens", "completion_tokens", "step_trigger", "transition"]


def span_for(week, row, session):
    key = f'{REVISION}:{week}:{row["session_id"]}:{row["step"]}'
    digest = hashlib.sha256(key.encode()).hexdigest()
    attributes = {
        "gen_ai.operation.name": "chat",
        "gen_ai.request.model": row["model"],
        "gen_ai.provider.name": row["provider"],
        "gen_ai.conversation.id": f'{week}:{row["session_id"]}',
        "gen_ai.usage.input_tokens": row["prompt_tokens"],
        "gen_ai.usage.output_tokens": row["completion_tokens"],
        "a2context.organization.id": "freeinference",
        "a2context.user.id": session["user_id"],
        "study.step_trigger": row["step_trigger"],
        "study.input_transition": row["transition"],
    }
    span = {
        "traceId": digest[:32],
        "spanId": digest[32:48],
        "startTimeUnixNano": str(row["start_ms"] * 1_000_000),
        "attributes": [
            {"key": key, "value": {"stringValue": str(value)}}
            for key, value in attributes.items() if value is not None
        ],
    }
    if row["end_ms"] is not None:
        span["endTimeUnixNano"] = str(row["end_ms"] * 1_000_000)
    return json.dumps(span, separators=(",", ":")).encode()


def run(args):
    started = time.monotonic()
    by_harness = Counter()
    accepted = 0
    sessions = 0
    for request_path in args.shards:
        week = request_path.name.removesuffix("-requests.parquet")
        signatures = ParquetFile(request_path.with_name(week + "-sessions.parquet"))
        session_columns = ["session_id", "harness", "user_id"]
        lookup = {}
        for batch in signatures.iter_batches(columns=session_columns):
            for row in batch.to_pylist():
                if row["session_id"] in lookup:
                    raise ValueError(f"Duplicate session in {week}.")
                lookup[row["session_id"]] = row
        seen = set()
        queues = defaultdict(list)
        parquet = ParquetFile(request_path)
        for batch in parquet.iter_batches(batch_size=2_000, columns=COLUMNS):
            for row in batch.to_pylist():
                session = lookup[row["session_id"]]
                harness = session["harness"] or "unknown-harness"
                seen.add(row["session_id"])
                queue = queues[harness]
                queue.append(span_for(week, row, session))
                if len(queue) == BATCH_TRACES:
                    deliver(args.sidecar, harness, queue)
                    accepted += len(queue)
                    by_harness[harness] += len(queue)
                    queue.clear()
        for harness, queue in queues.items():
            if queue:
                deliver(args.sidecar, harness, queue)
                accepted += len(queue)
                by_harness[harness] += len(queue)
        sessions += len(seen)
        print(f"{week}: {len(seen)} sessions, {accepted} cumulative requests accepted", flush=True)
    return {
        "revision": REVISION,
        "shards": [path.name for path in args.shards],
        "sessions": sessions,
        "accepted_requests": accepted,
        "by_harness": dict(by_harness),
        "seconds": round(time.monotonic() - started, 2),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("shards", type=Path, nargs="+")
    args = parser.parse_args()
    origin = urlsplit(args.sidecar)
    if origin.scheme != "http" or origin.hostname not in {"127.0.0.1", "localhost"} or origin.path not in {"", "/"}:
        parser.error("--sidecar must be a loopback HTTP origin")
    if not all(path.name.endswith("-requests.parquet") for path in args.shards):
        parser.error("expected files named WEEK-requests.parquet with adjacent WEEK-sessions.parquet")
    args.sidecar = args.sidecar.rstrip("/")
    args.shards.sort()
    summary = json.dumps(run(args), indent=2) + "\n"
    args.report.write_text(summary, encoding="utf-8")
    print(summary, end="")


if __name__ == "__main__":
    main()
