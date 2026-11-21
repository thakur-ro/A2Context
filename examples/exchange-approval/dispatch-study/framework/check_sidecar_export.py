"""Exercise the native batch exporter and verify ingested traces through the sidecar API."""
import argparse
from collections import Counter
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlsplit

from replay import HERE, logger, run
from telemetry import instrumentation

ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from check_sidecar import get, server


def paged(base, view):
    offset = 0
    while offset is not None:
        page = get(base, view, workflow="qwen-agent-exchange-replay", offset=offset)
        yield from page["rows"]
        offset = page["next_offset"]


def verify(binary):
    with TemporaryDirectory() as directory:
        with server(binary, Path(directory) / "spans.sqlite", "--capture-content") as base:
            with instrumentation(urlsplit(base).port) as tracer:
                result = run(tracer)
            if result != json.loads((HERE / "results.json").read_text()):
                raise ValueError("Observed framework receipts differ from the saved run.")
            health = get(base, "health")
            threads = list(paged(base, "threads"))
            traces = list(paged(base, "traces"))
            expected_threads = {r["case"] for r in result["rows"]}
            if {r["thread"] for r in threads} != expected_threads or len(traces) != 80:
                raise ValueError(f"Unexpected thread population or trace count: {threads}, {len(traces)}, {health}")
            counts = Counter()
            for row in traces:
                spans = get(base, "trace", id=row["id"])["trace"]["spans"]
                by_name = {s["name"]: s for s in spans}
                if len(by_name) != len(spans):
                    raise ValueError(f"Expected distinct span names in the captured trace: {spans}")
                counts.update(by_name.keys())
                root = by_name["Run Qwen-Agent"]
                for child in ("Replay saved assistant completion", "Run registered tool"):
                    if child in by_name and by_name[child].get("parent_span_id") != root["id"]:
                        raise ValueError(f"Unexpected parent for {child}: {by_name[child]}")
                if "Dispatch to SQLite" in by_name and by_name["Dispatch to SQLite"].get("parent_span_id") != by_name["Run registered tool"]["id"]:
                    raise ValueError("SQLite dispatch is missing its tool callback parent.")
            expected = {"Run Qwen-Agent": 80, "Replay saved assistant completion": 80,
                        "Run registered tool": 60, "Dispatch to SQLite": 16}
            coverage = {"traces": 80, "inference_spans": 80, "with_inputs": 0, "with_request": 0, "with_rules": 0}
            if counts != expected or health["coverage"] != coverage:
                raise ValueError(f"Stored spans or replay calls differ: {counts}, {health}")
            print(json.dumps({"threads": len(threads), "traces": len(traces), "spans": dict(counts),
                              "coverage": health["coverage"]}, indent=2))


if __name__ == "__main__":
    logger.setLevel("ERROR")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target" / "debug" / "a2context")
    verify(parser.parse_args().binary.resolve())
