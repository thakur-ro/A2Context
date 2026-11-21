"""Count tool catalogs and proposals in public Exgentic v2 runs."""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from pyarrow.parquet import ParquetFile


def scan(paths):
    totals = defaultdict(Counter)
    widths = defaultdict(Counter)
    missing = defaultdict(Counter)
    for path in paths:
        parquet = ParquetFile(path)
        for batch in parquet.iter_batches(batch_size=8, columns=["benchmark", "spans"]):
            for row in batch.to_pylist():
                workflow = row["benchmark"]
                counter = totals[workflow]
                counter["sessions"] += 1
                for span in row["spans"]:
                    attributes = span["attributes"]
                    counter["calls"] += 1
                    raw_catalog = attributes["gen_ai.tool.definitions"]
                    catalog = json.loads(raw_catalog) if raw_catalog else []
                    names = {entry["name"] for entry in catalog if isinstance(entry, dict) and entry.get("name")}
                    if names:
                        counter["calls_with_named_catalog"] += 1
                        widths[workflow][len(names)] += 1
                    raw_outputs = attributes["gen_ai.output.messages"]
                    outputs = json.loads(raw_outputs) if raw_outputs else []
                    proposals = {
                        part["name"]
                        for message in outputs or []
                        for part in message.get("parts") or []
                        if part.get("type") == "tool_call" and part.get("name")
                    }
                    if proposals:
                        counter["calls_with_explicit_proposals"] += 1
                    if names and proposals:
                        counter["calls_with_catalog_and_proposal"] += 1
                        absent = proposals - names
                        if absent:
                            counter["calls_with_proposal_outside_catalog"] += 1
                            missing[workflow].update(absent)
        print(path.name, "finished", flush=True)
    return {
        workflow: {
            **totals[workflow],
            "maximum_tools_in_one_call": max(widths[workflow], default=0),
            "catalog_sizes": dict(sorted(widths[workflow].items())),
            "outside_catalog_name_count": len(missing[workflow]),
            "most_frequent_outside_catalog_names": missing[workflow].most_common(5),
        }
        for workflow in sorted(totals)
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("shards", type=Path, nargs="+")
    args = parser.parse_args()
    result = scan(sorted(args.shards))
    args.report.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
