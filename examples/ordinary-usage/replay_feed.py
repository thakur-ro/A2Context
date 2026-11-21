"""Send the original pilot's captured spans into an empty loopback sidecar."""

import argparse
import json
from pathlib import Path
import sqlite3
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_sidecar import attrs, get, send_traces


def recorded_spans(directory):
    rows = []
    for workflow in ("papers", "tools"):
        source = (directory / workflow / "sidecar.sqlite").resolve()
        with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as db:
            for trace, raw in db.execute("SELECT id,raw FROM traces"):
                for call, (_, record) in json.loads(raw).items():
                    span = {"traceId": trace.removeprefix("otlp:"), "spanId": call, "name": record["name"],
                            "startTimeUnixNano": str(record["start"]), "endTimeUnixNano": str(record["end"]),
                            "attributes": attrs(record["attrs"])}
                    rows.append((record["end"], workflow, span))
    return sorted(rows, key=lambda row: (row[0], row[2]["spanId"]))


def replay(directory, base):
    if get(base, "traces")["rows"]:
        raise ValueError("Start a separate sidecar with an empty database for this replay.")
    rows = recorded_spans(directory)
    first = {}
    for index, (_, workflow, span) in enumerate(rows, 1):
        send_traces(base, {"resourceSpans": [{"resource": {"attributes": attrs({"service.name": workflow})},
                                              "scopeSpans": [{"spans": [span]}]}]})
        for card in get(base, "findings")["rows"]:
            first.setdefault(card["kind"], index)
    for card in get(base, "findings")["rows"]:
        evidence = card["evidence"]
        trace, call = evidence["trace_id"], evidence["call_id"]
        actual = get(base, "trace", id=trace, call=call)
        assert any(f["call_id"] == call and f["kind"] == card["kind"] for f in actual["findings"])
        print(json.dumps({"kind": card["kind"], "total_spans": len(rows), "first_seen_after_span": first[card["kind"]],
                          "conversations": card["threads"], "occurrences": card["occurrences"],
                          "trace": trace, "call": call, "comparison": card.get("discovery_example")}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    replay(args.directory, f"http://127.0.0.1:{args.port}")
