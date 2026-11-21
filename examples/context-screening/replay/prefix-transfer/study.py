"""Run or inspect the eight counterbalanced room-booking conditions."""
import argparse
from hashlib import sha256
from importlib import import_module
from itertools import product
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from check_question_holdout import compact, verify_model
from test_target_tokens import ROOT, run_probe

field = import_module("prefix-study.study").field
PROTOCOL = HERE / "protocol.json"


def design(protocol):
    sources = list(protocol["sources"])
    for first, atlas_minutes, requested in product(sources, protocol["minutes"], sources):
        order = [first, next(source for source in sources if source != first)]
        minutes = dict(zip(sources, [atlas_minutes, next(v for v in protocol["minutes"] if v != atlas_minutes)]))
        yield {"id": f"{first}-{atlas_minutes}-{requested}", "order": order, "requested": requested, "minutes": minutes}


def request_case(protocol, cell):
    entries = []
    for source in cell["order"]:
        name, minutes = protocol["sources"][source], cell["minutes"][source]
        entries.append((source, f"{minutes}m", protocol["entry"].format(name=name, minutes=minutes),
                        protocol["replacement"].format(name=name)))
    question = protocol["user"].format(name=protocol["sources"][cell["requested"]])
    return request_from_entries(protocol, entries, question)


def request_from_entries(protocol, entries, question):
    system = protocol["system"]
    declarations, edits = [], []
    for source, version, entry, replacement in entries:
        system += "\n"
        start = len(system.encode())
        system += entry
        location = {"pointer": "/messages/0/content", "text_range": {"start_byte": start, "end_byte": len(system.encode())}}
        declarations.append({"id": source, "source_id": source, "source_version": version, "request_fragment": location})
        edits.append({"context_id": "request-source:" + source, **location, "operation": "replace_text_fragment",
                      "replacement": replacement})
    body = {"model": protocol["checkpoint"], "messages": [{"role": "system", "content": system},
            {"role": "user", "content": question}], "temperature": protocol["temperature"], "max_tokens": protocol["max_tokens"]}
    return {"version": 1, "attributes": {"gen_ai.request.model": body["model"], "a2context.request.body": compact(body),
            "a2context.context.semantics": {"version": 1, "request_sources": declarations}}}, edits


def log_probs(report):
    conditions = report["scores"]["conditions"]
    by_id = {c["id"]: c["log_probs"] for c in conditions}
    return {"baseline": by_id["baseline"], **{e["source_id"]: by_id[e["condition_id"]] for e in report["effects"]}}


def measure(protocol, model, binary, cells, prepare, protocol_path):
    verify_model(model)
    if sha256((model / "tokenizer.json").read_bytes()).hexdigest() != protocol["tokenizer_sha256"]:
        raise ValueError("Tokenizer digest differs from the frozen protocol.")
    score = [ROOT / "scripts/score_qwen_candidates.py", model, "--model-id", protocol["checkpoint"], "--revision", protocol["revision"]]
    proposal = [ROOT / "scripts/propose_qwen.py", model, "--model-id", protocol["checkpoint"]]
    records = []
    for cell in cells:
        request, edits = prepare(protocol, cell)
        probe = {"version": 1, "id": "room-duration", "scorer_id": "qwen2-transformers-continuation-v1",
                 "candidates": [{"id": value, "continuation": value} for value in protocol["candidates"]], "interventions": edits}
        empty = run_probe(binary, "probe-candidates", request, probe, score)
        probe["assistant_prefix"] = protocol["prefix"]
        supplied = run_probe(binary, "probe-candidates", request, probe, score)
        if any(r["scores"]["runtime"] != protocol["runtime"] for r in [empty, supplied]):
            raise ValueError("Use the CPU runtime specified in the protocol.")
        expected = protocol["prefix"] + str(cell["minutes"][cell["requested"]]) + " minutes"
        spec = {"version": 1, "id": "room-answer", "runner_id": "Qwen2.5-0.5B-Instruct eaa56b5, Transformers 4.57.6 CPU, greedy",
                "expected": {"kind": "model_output", "label": "Model answer", "outcome": expected}, "samples": 1, "interventions": edits}
        shadow = run_probe(binary, "shadow", request, spec, proposal)
        envelope = {"request": request, "candidate": supplied, "shadow": shadow}
        records.append({**cell, "protocol_sha256": sha256(protocol_path.read_bytes()).hexdigest(), "empty_log_probs": log_probs(empty),
                        "envelope": envelope})
        print("Completed", cell["id"], file=sys.stderr, flush=True)
    return records


def outcomes(report):
    arms = {"baseline": report["baseline"], **{e["source_id"]: e["trials"] for e in report["effects"]}}
    answers = {}
    for source, trials in arms.items():
        trial, = trials
        answers[source] = trial.get("choice", {}).get("outcome")
    return answers


def verified_review(protocol, cell, record, binary, prepare, protocol_path):
    if any(cell[k] != record[k] for k in cell) or record["protocol_sha256"] != sha256(protocol_path.read_bytes()).hexdigest():
        raise ValueError("Saved records do not match the frozen conditions.")
    envelope = record["envelope"]
    request, edits = prepare(protocol, cell)
    if envelope["request"] != request:
        raise ValueError("Saved request differs from the declared condition.")
    candidate = envelope["candidate"]
    if ([effect["intervention"] for effect in candidate["effects"]] != edits
            or candidate["assistant_prefix"] != protocol["prefix"]
            or [c["continuation"] for c in candidate["candidates"]] != protocol["candidates"]):
        raise ValueError("Candidate conditions differ from the frozen intervention or readout.")
    for name, pinned in [("model_id","checkpoint"), ("model_revision","revision"),
                         ("runtime","runtime"), ("tokenizer_sha256","tokenizer_sha256")]:
        if candidate["scores"][name] != protocol[pinned]:
            raise ValueError(f"Saved {name} differs from the scoring protocol.")
    verified = json.loads(subprocess.check_output([binary, "inspect-pending", "-"], input=json.dumps(envelope), text=True))
    review, = verified["prefix_reviews"]
    run, = review["runs"]
    if any(row is None for row in run["rows"]) or len(run["rows"]) != 3:
        raise ValueError("A source edit is missing a matching trial condition.")
    return review


def summarize(protocol, records, binary):
    selectors = ["empty", "supplied", *protocol["sources"], "first", "last", "requested"]
    counts = {s: dict(selected=0, resolved=0, unresolved=0, switches=0) for s in selectors}
    comparisons = {p: dict(tp=0, fp=0, fn=0, tn=0, unresolved=0) for p in ["empty", "supplied"]}
    correctness = {a: dict(correct=0, wrong=0, unresolved=0, completed=0, followed_prefix=0)
                   for a in ["baseline", "edit_requested", "edit_other"]}
    any_switch = 0
    cells = list(design(protocol))
    if len(records) != len(cells):
        raise ValueError("Saved records must cover every frozen condition once.")
    for cell, record in zip(cells, records):
        review = verified_review(protocol, cell, record, binary, request_case, PROTOCOL)
        envelope = record["envelope"]
        answers = outcomes(envelope["shadow"])
        fields = {k: field(answer) for k, answer in answers.items()}
        changes = {s: fields[s] != fields["baseline"] if fields[s] and fields["baseline"] else None for s in cell["order"]}
        any_switch += True in changes.values()
        requested = cell["requested"]
        for source, answer in answers.items():
            arm = {"baseline": "baseline", requested: "edit_requested"}.get(source, "edit_other")
            key = "unresolved"
            if fields[source] is not None:
                key = "correct" if int(fields[source]) == cell["minutes"][requested] else "wrong"
            correctness[arm][key] += 1
            correctness[arm]["completed"] += answer is not None
            correctness[arm]["followed_prefix"] += answer is not None and answer.startswith(protocol["prefix"])
        selected = {s: [s] for s in cell["order"]}
        selected.update(first=cell["order"][:1], last=cell["order"][1:], requested=[cell["requested"]])
        for name, scores in [("empty", record["empty_log_probs"]), ("supplied", log_probs(envelope["candidate"]))]:
            margins = {source: a - b for source, (a, b) in scores.items()}
            selected[name] = [s for s in cell["order"] if margins[s] * margins["baseline"] < 0]
            for source, changed in changes.items():
                if changed is None:
                    key = "unresolved"
                elif source in selected[name]:
                    key = "tp" if changed else "fp"
                else:
                    key = "fn" if changed else "tn"
                comparisons[name][key] += 1
        if selected["supplied"] != [review["conditions"][i]["label"] for i in review["reversals"]]:
            raise ValueError("Study and Rust preference reversals disagree.")
        for name, choice in selected.items():
            if len(choice) == 1:
                changed = changes[choice[0]]
                counts[name]["selected"] += 1
                counts[name]["resolved"] += changed is not None
                counts[name]["unresolved"] += changed is None
                counts[name]["switches"] += changed is True
    return {"cells": len(records), "any_switch": any_switch, "selectors": counts, "edit_comparisons": comparisons,
            "original_value_and_prefix": correctness}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, help="Measure the frozen conditions with the pinned local checkpoint.")
    parser.add_argument("--cases", type=Path, default=HERE / "cases.json")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/a2context")
    args = parser.parse_args()
    protocol = json.loads(PROTOCOL.read_text())
    binary = args.binary.resolve()
    if args.model:
        if args.cases.exists():
            parser.error("Choose a new path for measured cases.")
        cases = measure(protocol, args.model.resolve(), binary, design(protocol), request_case, PROTOCOL)
        result = summarize(protocol, cases, binary)
        with args.cases.open("x") as handle:
            json.dump(cases, handle, indent=2, ensure_ascii=False, allow_nan=False)
            handle.write("\n")
    else:
        result = summarize(protocol, json.loads(args.cases.read_text()), binary)
    print(json.dumps(result, indent=2))
