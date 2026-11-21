"""Measure when logged action failures become visible in Who&When Pro text traces."""

import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass
from hashlib import sha256
import json
from pathlib import Path
from statistics import mean

REVISION = "0bd196c8a040841c4ae167ab33cc8151de246f1f"
TEXT_SHA256 = "51403bfd1b6570d0a9f8e62d25a50aa62504cb1264c33a3627ead893b7e2dd51"
FRAMEWORKS = {"alfagent", "smolagents"}
DEVELOPMENT = {"dabench", "dacode"}
SIGNALS = ("previous_rejection", "retry_after_rejection", "current_rejection")


@dataclass(frozen=True)
class Step:
    index: int
    coordinate: int
    action: str
    rejection: str | None


def recorded_steps(framework, trajectory):
    action_key = "action" if framework == "alfagent" else "code"
    cursor = None
    for index, entry in enumerate(trajectory):
        kind = entry["kind"]
        if kind in {"user", "final_answer"}:
            continue
        if kind not in {"action", "planning"}:
            raise ValueError(f"Unexpected event kind in {framework}: {kind}")
        if cursor is None:
            cursor = 0 if kind == "planning" else 1
        coordinate = entry["step_number"] if framework == "alfagent" else cursor
        if type(coordinate) is not int:
            raise ValueError("Expected an integer step coordinate")
        rejection = None
        if framework == "alfagent" and (entry.get("observation") or "").strip() == "Nothing happens.":
            rejection = "observation"
        if framework == "smolagents" and (entry.get("error") or {}).get("type"):
            rejection = "error"
        yield Step(index, coordinate, (entry.get(action_key) or "").strip(), rejection)
        cursor += 1


def evidence(steps):
    previous = None
    last_attempt = {}
    for step in steps:
        earlier = last_attempt.get(step.action)
        if previous and previous.rejection:
            yield "previous_rejection", step, previous
        if earlier and earlier.rejection:
            yield "retry_after_rejection", step, earlier
        if step.rejection:
            yield "current_rejection", step, step
        if step.action:
            last_attempt[step.action] = step
        previous = step


def digest(value):
    return sha256(value.encode()).hexdigest()


def summarize(cases):
    rejection_times = Counter()
    for case in cases:
        gold = case["gold"]
        for witness in case["candidates"]["current_rejection"]:
            coordinate = witness["candidate"]
            relation = "before" if coordinate < gold else "after" if coordinate > gold else "at"
            rejection_times[relation] += 1
    return {
        "traces": len(cases),
        "tasks": len({case["task"] for case in cases}),
        "steps": sum(case["steps"] for case in cases),
        "first_step_hits": sum(case["gold_index"] == 0 for case in cases),
        "last_step_hits": sum(case["gold_index"] == case["steps"] - 1 for case in cases),
        "uniform_hit_at_one": mean(1 / case["steps"] for case in cases),
        "uniform_hit_at_five": mean(min(5, case["steps"]) / case["steps"] for case in cases),
        "rejections_relative_to_label": {time: rejection_times[time] for time in ("before", "at", "after")},
        "signals": {signal: signal_metrics(cases, signal) for signal in SIGNALS},
    }


def signal_metrics(cases, signal):
    surfaced = first = top_five = found = marks = 0
    task_hits = defaultdict(list)
    for case in cases:
        candidates = case["candidates"][signal]
        positions = [witness["candidate"] for witness in candidates]
        gold = case["gold"]
        hit = gold in positions[:1]
        task_hits[case["task"]].append(hit)
        surfaced += bool(positions)
        marks += len(positions)
        first += hit
        top_five += gold in positions[:5]
        found += gold in positions
    return {"traces_surfaced": surfaced, "steps_marked": marks, "hit_at_one": first,
            "hit_at_five": top_five, "gold_steps_marked": found,
            "task_macro_hit_at_one": mean(mean(hits) for hits in task_hits.values())}


def prepare(archive):
    checksum = sha256()
    rows = []
    counts = Counter()
    with archive.open("rb") as stream:
        for line in stream:
            checksum.update(line)
            row = json.loads(line)
            framework = row["framework"]
            counts[framework] += 1
            if framework not in FRAMEWORKS:
                continue
            task = json.loads(row["task"])["query"]
            task_id = digest(" ".join(task.split()).casefold())
            trajectory = json.loads(row["trajectory"])
            steps = list(recorded_steps(framework, trajectory))
            coordinates = [step.coordinate for step in steps]
            gold = json.loads(row["ground_truth"])
            if list(sorted(set(coordinates))) != coordinates or gold["step"] not in coordinates:
                raise ValueError(f"Unaligned annotated step for {row['id']}")
            candidates = {signal: [] for signal in SIGNALS}
            for signal, candidate, source in evidence(steps):
                source_value = trajectory[source.index][source.rejection]
                witness = {"candidate": candidate.coordinate, "candidate_index": candidate.index,
                           "source": source.coordinate, "source_index": source.index,
                           "field": source.rejection,
                           "sha256": digest(json.dumps(source_value, ensure_ascii=False, sort_keys=True))}
                candidates[signal].append(witness)
            rows.append({"id": row["id"], "framework": framework, "benchmark": row["benchmark"],
                         "task": task_id, "steps": len(steps), "gold": gold["step"],
                         "gold_index": coordinates.index(gold["step"]), "mode": gold["mode"],
                         "candidates": candidates})
    if checksum.hexdigest() != TEXT_SHA256:
        raise ValueError("Text archive does not match the pinned release")
    if len({case["id"] for case in rows}) != len(rows):
        raise ValueError("Duplicate trace IDs")
    return counts, rows


def run(archive):
    counts, rows = prepare(archive)
    development = [case for case in rows if case["benchmark"] in DEVELOPMENT]
    dev_tasks = {case["task"] for case in development}
    held_out = defaultdict(list)
    overlap = 0
    for case in rows:
        if case["benchmark"] in DEVELOPMENT:
            continue
        if case["task"] in dev_tasks:
            overlap += 1
            continue
        held_out[case["benchmark"]].append(case)
    transfer = [case for group in held_out.values() for case in group]
    return {"revision": REVISION, "archive_sha256": TEXT_SHA256, "archive_framework_counts": counts,
            "excluded_tasks_in_development": overlap, "development": summarize(development),
            "transfer": summarize(transfer),
            "transfer_by_workflow": {name: summarize(group) for name, group in held_out.items()},
            "cases": development + transfer}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    report = run(args.archive)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    for scope in ("development", "transfer"):
        print(scope, json.dumps(report[scope], sort_keys=True))


if __name__ == "__main__":
    main()
