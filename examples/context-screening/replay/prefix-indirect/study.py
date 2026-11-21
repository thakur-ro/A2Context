"""Measure duration edits for questions that refer to activities by paraphrase."""
import argparse
from importlib import import_module
from itertools import product
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
room = import_module("prefix-transfer.study")
PROTOCOL = HERE / "protocol.json"


def design(protocol):
    sources, activities = list(protocol["sources"]), list(protocol["activities"])
    for first, minutes, atlas_activity, question in product(sources, protocol["minutes"], activities, activities):
        order = [first, next(s for s in sources if s != first)]
        durations = dict(zip(sources, [minutes, next(n for n in protocol["minutes"] if n != minutes)]))
        listings = dict(zip(sources, [atlas_activity, next(a for a in activities if a != atlas_activity)]))
        requested = next(s for s in sources if listings[s] == question)
        yield {"id": f"{first}-{minutes}-{atlas_activity}-{question}", "order": order, "minutes": durations,
               "activities": listings, "question_activity": question, "requested": requested}


def entries(protocol, cell):
    for source in cell["order"]:
        activity = cell["activities"][source]
        values = {"name": protocol["sources"][source], "activity": protocol["activities"][activity],
                  "minutes": cell["minutes"][source]}
        yield (source, f"{activity}-{values['minutes']}m", protocol["entry"].format(**values),
               protocol["replacement"].format(**values))


def prepare(protocol, cell):
    return room.request_from_entries(protocol, entries(protocol, cell), protocol["questions"][cell["question_activity"]])


def static_choices(protocol, cell):
    question = protocol["questions"][cell["question_activity"]].lower()
    words = set(re.findall(r"[a-z]+", question))
    selected = {s: [s] for s in cell["order"]}
    selected["first"], selected["last"] = cell["order"][:1], cell["order"][1:]
    selected["name"] = [s for s in cell["order"] if protocol["sources"][s].lower() in words]
    overlap, meaning = {}, []
    for source, _, entry, _ in entries(protocol, cell):
        overlap[source] = len(words.intersection(re.findall(r"[a-z]+", entry.lower())))
        for clue, activity in [("tropical plants", "greenhouse tour"), ("musicians", "orchestra rehearsal")]:
            if clue in question and activity in entry:
                meaning.append(source)
    selected["overlap"] = [s for s in overlap if overlap[s] == max(overlap.values())]
    selected["meaning"] = meaning
    return selected


def grade(value, expected):
    if value is None:
        return "unresolved"
    return "correct" if int(value) == expected else "wrong"


def evaluate_answers(answers, cell, protocol, arms, source_counts):
    fields = {source: room.field(answer) for source, answer in answers.items()}
    requested = cell["requested"]
    grades = {source: grade(value, cell["minutes"][requested]) for source, value in fields.items()}
    switches = {}
    for source, answer in answers.items():
        arm = {"baseline": "baseline", requested: "edit_requested"}.get(source, "edit_other")
        arms[arm][grades[source]] += 1
        arms[arm]["completed"] += answer is not None
        arms[arm]["followed_prefix"] += answer is not None and answer.startswith(protocol["prefix"])
        if source == "baseline":
            continue
        changed = None
        if fields[source] is not None and fields["baseline"] is not None:
            changed = fields[source] != fields["baseline"]
        switches[source] = changed
        category = {True: "switched", False: "held", None: "unresolved"}[changed]
        source_counts[source][category] += 1
    return grades, switches


def evaluate_reversals(scores, order, switches, counts):
    margins = {source: left - right for source, (left, right) in scores.items()}
    reversals = [source for source in order if margins[source] * margins["baseline"] < 0]
    for source, changed in switches.items():
        if changed is None:
            category = "unresolved"
        elif source in reversals:
            category = "tp" if changed else "fp"
        else:
            category = "fn" if changed else "tn"
        counts[category] += 1
    return reversals


def count_selections(choices, counts, switches, grades):
    for name, chosen in choices.items():
        if len(chosen) != 1:
            continue
        source, = chosen
        row = counts[name]
        changed = switches[source]
        row["selected"] += 1
        row["before"][grades["baseline"]] += 1
        row["after"][grades[source]] += 1
        row["resolved"] += changed is not None
        row["unresolved"] += changed is None
        if changed:
            row["switches"] += 1
            row["correct_to_wrong"] += grades["baseline"] == "correct" and grades[source] == "wrong"
            row["wrong_to_correct"] += grades["baseline"] == "wrong" and grades[source] == "correct"


def summarize(protocol, records, binary):
    keys = ["empty", "supplied", *protocol["sources"], "first", "last", "name", "overlap", "meaning"]
    counts = {k: {"selected": 0, "resolved": 0, "unresolved": 0, "switches": 0,
                  "before": dict(correct=0, wrong=0, unresolved=0), "after": dict(correct=0, wrong=0, unresolved=0),
                  "correct_to_wrong": 0, "wrong_to_correct": 0} for k in keys}
    comparisons = {k: dict(tp=0, fp=0, tn=0, fn=0, unresolved=0) for k in ["empty", "supplied"]}
    arms = {k: dict(correct=0, wrong=0, unresolved=0, followed_prefix=0, completed=0)
            for k in ["baseline", "edit_requested", "edit_other"]}
    changed_by_source = {k: dict(switched=0, held=0, unresolved=0) for k in protocol["sources"]}
    pairs, any_switch = {}, 0
    cells = list(design(protocol))
    if len(cells) != len(records):
        raise ValueError("Expected a saved report for every condition.")
    for cell, record in zip(cells, records):
        review = room.verified_review(protocol, cell, record, binary, prepare, PROTOCOL)
        envelope = record["envelope"]
        answers = room.outcomes(envelope["shadow"])
        grades, switches = evaluate_answers(answers, cell, protocol, arms, changed_by_source)
        any_switch += True in switches.values()
        choices = static_choices(protocol, cell)
        for kind, scores in [("empty", record["empty_log_probs"]), ("supplied", room.log_probs(envelope["candidate"]))]:
            choices[kind] = evaluate_reversals(scores, cell["order"], switches, comparisons[kind])
        if choices["supplied"] != [review["conditions"][i]["label"] for i in review["reversals"]]:
            raise ValueError("The verifier and the study disagree on the strict preference reversal.")
        count_selections(choices, counts, switches, grades)
        key = cell["order"][0], cell["minutes"]["atlas-calendar"], cell["activities"]["atlas-calendar"]
        one = choices["supplied"][0] if len(choices["supplied"]) == 1 else None
        pairs.setdefault(key, []).append(one if switches.get(one) is True else None)
    paired_hits = sum(None not in chosen and len(set(chosen)) == 2 for chosen in pairs.values())
    return {"cells": len(records), "cells_with_a_switch": any_switch, "pairs": len(pairs),
            "pairs_with_different_verified_selections": paired_hits, "selectors": counts,
            "field_switch_by_source": changed_by_source, "reversal_vs_numeric_switch": comparisons, "outcomes": arms}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path)
    parser.add_argument("--cases", type=Path, default=HERE / "cases.json")
    parser.add_argument("--binary", type=Path, default=room.ROOT / "target/debug/a2context")
    args = parser.parse_args()
    protocol = json.loads(PROTOCOL.read_text())
    binary = args.binary.resolve()
    if args.model:
        if args.cases.exists():
            parser.error("Choose a new output path for this measurement.")
        records = room.measure(protocol, args.model.resolve(), binary, design(protocol), prepare, PROTOCOL)
        result = summarize(protocol, records, binary)
        with args.cases.open("x") as handle:
            json.dump(records, handle, indent=2, ensure_ascii=False, allow_nan=False)
            handle.write("\n")
    else:
        result = summarize(protocol, json.loads(args.cases.read_text()), binary)
    print(json.dumps(result, indent=2))
