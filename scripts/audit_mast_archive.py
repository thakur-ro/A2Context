"""Check trace identities, duplicate trajectories, and human-label joins in MAST."""

import argparse
from collections import Counter, defaultdict
from hashlib import sha256
import json
from pathlib import Path

REVISION = "95118ac951421753cf1deb87ddea3b01e693c41b"
ARCHIVES = {
    "MAD_full_dataset.json": "d636ac63dfc1c6af2d312e862f4b7d383b62d3898d431ccd0e7a79d21d85406f",
    "MAD_human_labelled_dataset.json": "30a0c4075078e9a1b8c39bc608d2b5156cc64c6bda1f6fd262786eb81ff4a286",
}


def digest(text):
    return sha256(text.encode()).hexdigest()


def public_identity(row):
    return row["mas_name"], row["benchmark_name"], str(row["trace_id"])


def load(directory, name):
    data = (directory / name).read_bytes()
    if sha256(data).hexdigest() != ARCHIVES[name]:
        raise ValueError(f"{name} does not match MAST revision {REVISION}.")
    return json.loads(data)


def audit(traces, human):
    framework_ids = Counter(public_identity(row) for row in traces)
    model_ids = Counter((*public_identity(row), row["llm_name"]) for row in traces)
    published_paths = set()
    by_body = defaultdict(list)
    for row in traces:
        trace = row["trace"]
        identity = (*public_identity(row), row["llm_name"], trace["key"], trace["index"])
        if identity in published_paths:
            raise ValueError("The complete published identity is ambiguous.")
        published_paths.add(identity)
        by_body[digest(trace["trajectory"])].append(identity)
    by_human_count = Counter(len(by_body.get(digest(row["trace"]), ())) for row in human)
    duplicate_groups = [{"body_sha256": key, "identities": rows} for key, rows in sorted(by_body.items()) if len(rows) > 1]
    return {
        "revision": REVISION,
        "annotated_rows": len(traces),
        "model_rows_by_framework": Counter(row["mas_name"] for row in traces),
        "model_rows_by_benchmark": Counter(row["benchmark_name"] for row in traces),
        "unique_trajectory_bodies": len(by_body),
        "colliding_public_ids": sum(n > 1 for n in framework_ids.values()),
        "rows_beyond_unique_model_ids": len(traces) - len(model_ids),
        "human_rows": len(human),
        "human_joins_by_number_of_exact_matches": by_human_count,
        "duplicate_trajectory_groups": duplicate_groups,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    traces = load(args.directory, "MAD_full_dataset.json")
    human = load(args.directory, "MAD_human_labelled_dataset.json")
    result = audit(traces, human)
    args.report.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"Checked {result['annotated_rows']} annotated rows and {result['human_rows']} human rows.")


if __name__ == "__main__":
    main()
