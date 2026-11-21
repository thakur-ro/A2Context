"""Compare age-based heuristics with an origin revision in a controlled replay."""

import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass, replace
from datetime import datetime
from hashlib import sha256
import json
from pathlib import Path

from pyarrow.parquet import ParquetFile

from study_tool_result_retention import REVISION, observed_repeats

DEVELOPMENT = {"appworld", "browsecompplus", "tau2_retail"}
THRESHOLDS = (1, 5, 15, 30, 60, 120, 300)
ARMS = (
    "unchanged", "captured_update", "no_op_write", "missing_revision",
    "other_scope", "concurrent_update", "corrupted_cache",
)


def observations(paths):
    columns = ["benchmark", "session_id", "harness", "spans"]
    for path in paths:
        for batch in ParquetFile(path).iter_batches(batch_size=1, columns=columns):
            row = batch.to_pylist()[0]
            for event in observed_repeats(row):
                old = event["previous_result"]
                new = event["later_result"]
                if not event["read_like_name"] or new is None or old["has_error_field"] or new["has_error_field"]:
                    continue
                age = datetime.fromisoformat(event["repeated_proposal"]["started_at"]) - datetime.fromisoformat(old["started_at"])
                yield {
                    "workflow": row["benchmark"],
                    "session": row["session_id"],
                    "age_seconds": age.total_seconds(),
                    **event,
                }
        print(path.name, "scanned", flush=True)


@dataclass(frozen=True)
class Read:
    scope: str
    revision: int | None
    digest: str

    def write(self, digest):
        return replace(self, revision=self.revision + 1, digest=digest)


def score(digest, current, served_from_cache):
    if not served_from_cache:
        return "lookup"
    return "unchanged" if digest == current.digest else "changed"


def simulate(row, threshold):
    """The origin advances its revision on writes, including writes with equal payloads."""
    earlier = row["previous_result"]["digest"]
    later = row["later_result"]["digest"]
    scope = row["session"] + "/" + row["repeated_proposal"]["invocation"]
    cached = Read(scope, 1, earlier)
    for arm in ARMS:
        observed = cached
        payload = sha256(("corrupted:" + earlier).encode()).hexdigest() if arm == "corrupted_cache" else earlier
        if arm == "captured_update":
            observed = cached.write(later)
        elif arm == "no_op_write":
            observed = cached.write(earlier)
        elif arm == "missing_revision":
            observed = replace(cached.write(later), revision=None)
        elif arm == "other_scope":
            # A revision number can coincide across resources or workspaces.
            observed = Read(scope + "/other", cached.revision, later)
        current = observed.write(later) if arm == "concurrent_update" else observed
        check_match = (cached.scope, cached.revision) == (observed.scope, observed.revision)
        atomic_match = (cached.scope, cached.revision) == (current.scope, current.revision)
        decisions = {
            "always_lookup": False,
            "always_reuse": True,
            "age": threshold is not None and row["age_seconds"] <= threshold,
            "compare_then_serve": check_match,
            "conditional_read_and_digest": atomic_match and payload == cached.digest,
        }
        yield arm, {policy: score(payload, current, decision) for policy, decision in decisions.items()}


def age_counts(rows, threshold):
    outcomes = Counter()
    for row in rows:
        if threshold is None or row["age_seconds"] > threshold:
            outcomes["lookup"] += 1
        elif row["result_comparison"] == "equal":
            outcomes["unchanged"] += 1
        else:
            outcomes["changed"] += 1
    return dict(outcomes)


def report(rows):
    development = [row for row in rows if row["workflow"] in DEVELOPMENT]
    held_out = [row for row in rows if row["workflow"] not in DEVELOPMENT]
    if not development or not held_out:
        raise ValueError("The study needs observations from both the development and transfer workflows.")
    candidates = {seconds: age_counts(development, seconds) for seconds in THRESHOLDS}
    passing = [seconds for seconds, counts in candidates.items()
               if counts.get("unchanged", 0) and not counts.get("changed", 0)]
    threshold = max(passing, key=lambda seconds: (candidates[seconds]["unchanged"], -seconds), default=None)
    # The held-out curve is descriptive. It cannot change the selected threshold.
    observed = {
        "development": candidates,
        "selected_seconds": threshold,
        "held_out": age_counts(held_out, threshold),
        "held_out_by_workflow": {
            workflow: age_counts([row for row in held_out if row["workflow"] == workflow], threshold)
            for workflow in sorted({row["workflow"] for row in held_out})
        },
        "held_out_curve": {seconds: age_counts(held_out, seconds) for seconds in THRESHOLDS},
    }
    simulations = defaultdict(lambda: defaultdict(Counter))
    for row in held_out:
        for arm, decisions in simulate(row, threshold):
            for policy, outcome in decisions.items():
                simulations[arm][policy][outcome] += 1
    samples = {}
    for row in rows:
        key = row["workflow"], row["result_comparison"]
        rank = sha256(f"{row['session']}:{row['repeated_proposal']['span']}".encode()).hexdigest()
        if key not in samples or rank < samples[key][0]:
            samples[key] = rank, row
    return {
        "revision": REVISION,
        "capture_population": Counter(row["workflow"] for row in rows),
        "sessions": {workflow: len({r["session"] for r in rows if r["workflow"] == workflow})
                     for workflow in sorted({row["workflow"] for row in rows})},
        "archive_age_comparison": observed,
        "simulated_transfer_arms": simulations,
        "examples": [samples[key][1] for key in sorted(samples)],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("shards", type=Path, nargs="+")
    args = parser.parse_args()
    result = report(list(observations(sorted(args.shards))))
    args.report.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print("Evaluated", sum(result["capture_population"].values()), "paired captured reads.", flush=True)


if __name__ == "__main__":
    main()
