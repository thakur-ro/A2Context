"""Evaluate which logged repeats surface traces labelled Step Repetition."""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re

from audit_mast_archive import REVISION, digest, load, public_identity
from mast_events import events_for

TRANSFER = {"Olympiad", "GAIA", "Test-C", "SWE-Bench-Lite"}
SCORES = ("raw_lines", "exact_event", "same_actor_exact", "same_actor_near")
TOKEN = re.compile(r"\w+|[^\w\s]")
NUMERIC = re.compile(r"(?<!\w)-?\d+(?:\.\d+)?(?!\w)")


def anchor(body, event):
    return {"actor": event.actor, "channel": event.channel, "start": event.start,
            "end": event.end, "excerpt_sha256": digest(body[event.start:event.end])}


def shingles(text):
    words = TOKEN.findall(text.casefold())
    return set(zip(words, words[1:], words[2:])) if len(words) >= 12 else set()


def similarity(left, right):
    if NUMERIC.findall(left) != NUMERIC.findall(right):
        return 0.0
    a, b = shingles(left), shingles(right)
    return len(a & b) / len(a | b) if a and b else 0.0


def features(framework, body):
    lines = [line for line in (x.strip() for x in body.splitlines()) if len(line) >= 24]
    events = [event for event in events_for(framework, body) if len(event.text) >= 24]
    exact = Counter(event.text for event in events)
    stats = {"log_length": len(body),
             "raw_lines": (len(lines) - len(set(lines))) / max(1, len(lines)),
             "exact_event": (len(events) - len(exact)) / max(1, len(events))}
    prior = {}
    identical = nearby = 0
    witness, best = None, 0.0
    for i, event in enumerate(events):
        lane = (event.actor, event.channel)
        previous = prior.get(lane)
        prior[lane] = i, event
        if previous is None or i - previous[0] > 8:
            continue
        earlier = previous[1]
        equal = event.text == earlier.text
        strength = 1.0 if equal else similarity(earlier.text, event.text)
        if strength >= 0.92:
            identical += int(equal)
            nearby += 1
            if strength > best:
                best = strength
                witness = {"strength": strength, "earlier": anchor(body, earlier), "later": anchor(body, event)}
    stats["same_actor_exact"] = identical / max(1, len(events))
    stats["same_actor_near"] = nearby / max(1, len(events))
    return stats, len(events), witness


def human_label(row):
    matches = [item for item in row["annotations"]
               if re.fullmatch(r"\d+\.\d+ step repetition", item["failure mode"].splitlines()[0].strip(), re.I)]
    if len(matches) != 1:
        raise ValueError("Expected one human annotation for Step Repetition by name.")
    votes = [matches[0][f"annotator_{i}"] for i in (1, 2, 3)]
    if any(not isinstance(v, bool) for v in votes):
        raise ValueError("Unexpected human annotation value.")
    return int(sum(votes) >= 2), sum(votes)


def prepare(machine, human):
    official, human_rows = defaultdict(list), []
    human_bodies = {digest(row["trace"]) for row in human}
    for row in machine:
        official[digest(row["trace"]["trajectory"])].append(row)
    exclusions = Counter()
    selected = []
    for body_id, group in sorted(official.items()):
        if body_id in human_bodies:
            exclusions["human_overlap"] += 1
            continue
        labels = {row["mast_annotation"]["1.3"] for row in group} - {None}
        if len(labels) != 1:
            exclusions["disputed_label" if labels else "missing_label"] += 1
            continue
        held_out = [row for row in group if row["benchmark_name"] in TRANSFER]
        row = min(held_out or group, key=lambda row: (*public_identity(row), row["llm_name"], row["trace"]["key"], row["trace"]["index"]))
        selected.append({"framework": row["mas_name"], "benchmark": row["benchmark_name"],
                         "model": row["llm_name"], "trace_id": row["trace_id"], "trace_key": row["trace"]["key"],
                         "trace_index": row["trace"]["index"], "body": row["trace"]["trajectory"],
                         "body_sha256": body_id, "label": labels.pop(), "cohort": "transfer" if held_out else "development"})
    for row in human:
        label, votes = human_label(row)
        human_rows.append({"framework": row["mas_name"], "benchmark": row["benchmark_name"], "trace_id": row["trace_id"],
                           "round": row["round"], "body": row["trace"], "body_sha256": digest(row["trace"]),
                           "label": label, "positive_votes": votes, "cohort": "human"})
    return selected + human_rows, exclusions


def score(rows, method):
    ranked = sorted(rows, key=lambda row: (-row["scores"][method], row["body_sha256"]))
    scored_by_tie = defaultdict(list)
    for row in ranked:
        scored_by_tie[row["scores"][method]].append(row)
    positive = sum(row["label"] for row in ranked)
    surfaced = [row for row in ranked if row["scores"][method] > 0]
    good = sum(row["label"] for row in surfaced)
    ap = true_positive = seen = 0
    for ties in scored_by_tie.values():
        hits = sum(row["label"] for row in ties)
        seen += len(ties)
        true_positive += hits
        ap += hits * true_positive / seen
    top = surfaced[:5]
    return {"traces": len(ranked), "positive": positive, "surfaced": len(surfaced), "true_positive": good,
            "precision": good / len(surfaced) if surfaced else None, "recall": good / positive if positive else None,
            "average_precision": ap / positive if positive else None,
            "top_five_reviewed": len(top), "top_five_positive": sum(row["label"] for row in top)}


def report(rows):
    methods = ("log_length", *SCORES)
    overall = {m: score(rows, m) for m in methods}
    by_framework = {key: {m: score([row for row in rows if row["framework"] == key], m) for m in methods}
                    for key in sorted({row["framework"] for row in rows})}
    return {"event_coverage": sum(row["events"] >= 2 for row in rows), "overall": overall, "by_framework": by_framework}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    rows, exclusions = prepare(load(args.directory, "MAD_full_dataset.json"), load(args.directory, "MAD_human_labelled_dataset.json"))
    for row in rows:
        body = row.pop("body")
        row["scores"], row["events"], row["witness"] = features(row["framework"], body)
    results = {cohort: report([row for row in rows if row["cohort"] == cohort])
               for cohort in ("development", "transfer", "human")}
    selected = max(SCORES, key=lambda method: results["development"]["overall"][method]["average_precision"])
    output = {"revision": REVISION, "label": "Step Repetition", "offset_unit": "unicode_codepoint",
              "transfer_benchmarks": sorted(TRANSFER),
              "excluded_groups": exclusions, "selected_on_development": selected, "cohorts": results, "traces": rows}
    args.report.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    for name, result in results.items():
        print(name, "event coverage", result["event_coverage"], "selected", selected, result["overall"][selected])


if __name__ == "__main__":
    main()
