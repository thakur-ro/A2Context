"""Time scoped routing card queries over 256 captured calls with four distinct inputs."""
import argparse
import json
from pathlib import Path
from statistics import median
import sys
from tempfile import TemporaryDirectory
from time import perf_counter

from prefill_cohort import ATTRIBUTE, export
from replicate_routing import RESULTS, SCRIPTS

sys.path.insert(0, str(SCRIPTS))
from check_sidecar import get, send_traces, server


def benchmark(binary, folder):
    records = json.loads(RESULTS.read_text())
    document = export(records, {r["id"]: r["extra"]["metadata"][ATTRIBUTE] for r in records})
    spans = document["resourceSpans"][0]["scopeSpans"][0]["spans"]
    with server(binary, folder / "store", "--capture-content") as base:
        for batch in range(64):
            for i, span in enumerate(spans):
                identifier = batch * len(spans) + i + 1
                span.update(traceId=f"{identifier:032x}", spanId=f"{identifier:016x}")
                thread = next(a for a in span["attributes"] if a["key"] == "gen_ai.conversation.id")
                thread["value"]["stringValue"] = f"conversation-{identifier}"
            send_traces(base, document)
        timings = []
        for i in range(21):
            start = perf_counter()
            card, = get(base, "findings", org="retail-south", kind="prefill")["rows"]
            elapsed = (perf_counter() - start) * 1000
            assert card["threads"] == card["traces"] == 256
            if "routing_profiles" in card:
                assert card["routing_conversations"] == 256 and sum(p["inputs"] for p in card["routing_profiles"]) == 4
            if i:
                timings.append(elapsed)
        return {"requests": 256, "distinct_inputs": 4, "timed_reads": len(timings),
                "median_ms": round(median(timings), 2), "p95_ms": round(sorted(timings)[18], 2),
                "queue_drops": get(base, "health")["queue_drops"]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with TemporaryDirectory() as folder:
        print(json.dumps(benchmark(args.binary.resolve(), Path(folder))))
