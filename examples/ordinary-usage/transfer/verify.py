"""Resolve the review against the original transfer captures and application logs."""

import json
from pathlib import Path
import re
import sqlite3
import sys

from record import record


def verify(root):
    saved = json.loads(Path(__file__).with_name("outcomes.json").read_text())
    assert record(root) == saved
    with sqlite3.connect((root / "sql/history.db").as_uri() + "?mode=ro", uri=True) as db:
        days, = db.execute("SELECT count(DISTINCT date) FROM commits JOIN changes USING(sha) "
                           "WHERE path LIKE 'src/sidecar/%'").fetchone()
        assert days == 3
    for workflow, reports in saved.items():
        assert reports["distinct_findings"] == 0 and not reports["capture_errors"]
        assert all(c["http"] == 200 and c["output_captured"] for s in reports["sessions"] for c in s["calls"])
        folder = root / workflow
        with sqlite3.connect((folder / "sidecar.sqlite").as_uri() + "?mode=ro", uri=True) as db:
            for name, raw in db.execute("SELECT thread,run FROM traces"):
                calls = sorted(json.loads(raw)["calls"], key=lambda c: c["started_at"])
                final = calls[-1]
                request = json.loads(final["recorded_request"]["body"])
                log = (folder / (name + ".log")).read_text()
                decision, = final["decisions"]
                if workflow == "sql" and name != "sql-4":
                    tool, = [t["function"] for t in request["tools"] if t["function"]["name"] == decision["label"]]
                    assert decision["label"] == "sql_db_query" and "query" in tool["parameters"]["required"]
                    args = json.loads(decision["outcome"])
                    assert isinstance(args, dict) and "query" not in args
                    assert re.search(r"ValidationError:.*_QuerySQLDatabaseToolInput\nquery\n  Field required", log)
                elif name == "sql-4":
                    assert "count of 7 distinct days" in log
                    proposals = [d["label"] for c in calls for d in c["decisions"] if d["kind"] == "tool_call"]
                    assert "sql_db_query_checker" in proposals and "sql_db_query" not in proposals
                elif name in reports["quiet_sample"]:
                    prompt = request["messages"][-1]["content"]
                    example, body = prompt.split("Now it's your turn.", 1)
                    if name == "citations-1":
                        assert "sky is red" in log and "sky is red" in example and "sky" not in body.lower()
                    elif name == "citations-2":
                        assert "48.7 MiB" in log and "one CPU and 512 MiB" in body and "512" not in log
                    elif name == "citations-5":
                        assert "2021.12345" in log and "2021.12345" not in prompt
    print("Verified the frozen feed, tool rejections, database count and all six sampled evidence references.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
