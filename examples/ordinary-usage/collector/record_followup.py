"""Preserve the displayed card, actual application citations and retrieved paths for review."""

import argparse
from datetime import datetime
from hashlib import sha256
import json
from pathlib import Path
import sqlite3


def record(folder, reviewed_at):
    events = [json.loads(line) for line in (folder / "events.jsonl").read_text().splitlines()]
    captures = {e["trace"]: e for e in events if e["event"] == "capture"}
    first = next(e for e in events if e.get("cards"))
    rows = []
    with sqlite3.connect((folder / "sidecar.sqlite").resolve().as_uri() + "?mode=ro", uri=True) as db:
        for trace, raw in db.execute("SELECT id,run FROM traces ORDER BY start,id"):
            run = json.loads(raw)
            (call,) = run["calls"]
            event = captures[trace]
            name = run["conversation_id"]
            log = (folder / (name + ".log")).read_text()
            request = json.loads(call["recorded_request"]["body"])
            prompt = request["messages"][0]["content"]
            labels = [
                line.removeprefix("Source: ")
                for line in prompt.splitlines()
                if line.startswith("Source: ")
            ]
            # The adapter supplies exactly three retrieved chunks after the chain's worked examples.
            sources = log.rsplit("\nSources: ", 1)[1].strip()
            rows.append(
                {
                    "session": name,
                    "trace": trace,
                    "call": call["id"],
                    "http": event["status"],
                    "example_labels": labels[:-3],
                    "retrieved_labels": labels[-3:],
                    "application_sources": sources,
                    "reply": call["decisions"][0]["outcome"],
                    "log_sha256": sha256(log.encode()).hexdigest(),
                }
            )
    started = datetime.fromisoformat(events[0]["at"])
    displayed = datetime.fromisoformat(first["at"])
    review = datetime.fromisoformat(reviewed_at)
    assert review >= displayed >= started
    return {
        "frozen": "1d92630",
        "started": started.isoformat(),
        "displayed": displayed.isoformat(),
        "developer_reviewed": review.isoformat(),
        "first_evidence": first["cards"][0]["evidence"],
        "final_groups": [
            {k: c[k] for k in ("id", "kind", "occurrences", "threads")}
            for c in json.loads((folder / "findings.json").read_text())["rows"]
        ],
        "capture_errors": [e for e in events if e["event"] == "capture_error"],
        "application_exits": [e["status"] for e in events if e["event"] == "application_exit"],
        "sessions": rows,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    parser.add_argument(
        "--reviewed-at", required=True, help="Actual UTC time of the developer's evidence review"
    )
    args = parser.parse_args()
    print(json.dumps(record(args.folder, args.reviewed_at), indent=2))
