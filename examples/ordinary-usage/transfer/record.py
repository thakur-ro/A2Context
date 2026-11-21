"""Summarize a completed transfer run without rerunning the model."""

from hashlib import sha256
import json
from pathlib import Path
import sqlite3
import sys


def record(directory):
    summary = {}
    for workflow in ("sql", "citations"):
        folder = directory / workflow
        events = [json.loads(line) for line in (folder / "events.jsonl").read_text().splitlines()]
        sessions = []
        findings = json.loads((folder / "findings.json").read_text())["rows"]
        for start in (e for e in events if e["event"] == "started"):
            name = start["run"]
            captures = [e for e in events if e["run"] == name and e["event"] == "capture"]
            exit_event, = [e for e in events if e["run"] == name and e["event"] == "application_exit"]
            sessions.append({"session":name, "trace":start["trace"], "started":start["at"], "closed":exit_event["at"],
                             "exit":exit_event["status"], "calls":[{"span":e["span"],"http":e["status"],
                             "output_captured":e["captured_output"],"new_findings":[c["id"] for c in e["cards"]]}
                             for e in captures], "log_sha256":sha256((folder / (name + ".log")).read_bytes()).hexdigest()})
        # A group can cover several traces. Read the complete set before sampling.
        with sqlite3.connect((folder / "sidecar.sqlite").resolve().as_uri() + "?mode=ro", uri=True) as db:
            affected = {r[0] for r in db.execute("SELECT DISTINCT trace_id FROM observations")}
        quiet = sorted((s for s in sessions if s["trace"] not in affected), key=lambda s: sha256(s["trace"].encode()).digest())
        summary[workflow] = {"sessions":sessions, "score":json.loads((folder / "tool-reference-score.json").read_text()),
                             "distinct_findings":len(findings), "quiet_sample":[s["session"] for s in quiet[:3]],
                             "capture_errors":[e for e in events if e["event"] == "capture_error"]}
    return summary


if __name__ == "__main__":
    print(json.dumps(record(Path(sys.argv[1])), indent=2))
