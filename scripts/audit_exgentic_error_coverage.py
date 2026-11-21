"""Count earlier inference errors in successful and unsuccessful Exgentic sessions."""

import argparse
from collections import Counter, defaultdict
from datetime import datetime
import json
from pathlib import Path

from pyarrow.parquet import ParquetFile

COLUMNS = ["benchmark", "success", "spans.list.element.start_time", "spans.list.element.end_time",
           "spans.list.element.attributes.error.type", "spans.list.element.attributes.gen_ai.operation.name"]


def census(shards):
    workflows = defaultdict(lambda: defaultdict(Counter))
    error_types = Counter()
    operations = Counter()
    for shard in shards:
        for batch in ParquetFile(shard).iter_batches(batch_size=128, columns=COLUMNS):
            for row in batch.to_pylist():
                spans = row["spans"] or []
                counts = workflows[row["benchmark"]][str(row["success"]).lower()]
                counts["sessions"] += 1
                counts["spans"] += len(spans)
                latest = max((datetime.fromisoformat(span["start_time"]) for span in spans if span["start_time"]),
                             default=None)
                errors = [span for span in spans if (span["attributes"] or {}).get("error.type")]
                counts["sessions_with_error"] += bool(errors)
                counts["sessions_with_error_before_later_call"] += latest is not None and any(
                    span["end_time"] is not None and datetime.fromisoformat(span["end_time"]) < latest
                    for span in errors
                )
                error_types.update(span["attributes"]["error.type"] for span in errors)
                operations.update(str((span["attributes"] or {}).get("gen_ai.operation.name")) for span in spans)
    return {"workflows": workflows, "error_types": error_types, "operations": operations}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shards", type=Path, nargs="+")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    result = census(args.shards)
    args.report.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
