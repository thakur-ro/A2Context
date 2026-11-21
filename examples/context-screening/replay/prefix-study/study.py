"""Compare two supplied-prefix selectors with fields extracted from greedy answers."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from check_question_holdout import SOURCES, conditions, digest, load_runtime, propose, verify_model
from score_qwen_candidates import score_batch
from test_target_tokens import ROOT, run_probe

PROTOCOL = HERE / "protocol.json"
CANDIDATES = [{"id": value, "continuation": value} for value in ("30", "60")]


def save(path, data):
    with path.open("x") as handle:
        json.dump(data, handle, indent=2, ensure_ascii=False, allow_nan=False)
        handle.write("\n")


def prepare(protocol, question):
    reference = json.loads((HERE / protocol["reference"]).read_text())
    reference["request"]["max_tokens"] = protocol["max_tokens"]
    return conditions(reference, question["question"])


def measure(protocol, model):
    verify_model(model)
    tokenizer = sha256((model / "tokenizer.json").read_bytes()).hexdigest()
    runtime = load_runtime(model)
    if tokenizer != protocol["tokenizer_sha256"] or runtime.identity != protocol["runtime"]:
        raise ValueError("Use the tokenizer and CPU runtime specified in the protocol.")
    cases = []
    with runtime.torch.inference_mode():
        for question in protocol["questions"]:
            batch, _, _ = prepare(protocol, question)
            bodies = {"baseline": batch["baseline"]}
            bodies.update({edit["source_id"]: edit["request"] for edit in batch["edits"]})
            raw = {"version": 1, "candidates": CANDIDATES,
                   "conditions": [{"id": name, "request": body} for name, body in bodies.items()]}
            scores = {}
            for name, prefix in (("empty", ""), ("supplied", question["prefix"])):
                raw["assistant_prefix"] = prefix
                report = score_batch(runtime, raw, protocol["checkpoint"], protocol["revision"], tokenizer,
                                     include_steps=True)
                scores[name] = {row["id"]: row["log_probs"] for row in report["conditions"]}
            answers = {}
            for name, body in bodies.items():
                try:
                    answers[name] = propose(runtime, body, protocol["checkpoint"])["outcome"]
                except ValueError as error:
                    if "generation limit" not in str(error):
                        raise
                    answers[name] = None
            cases.append({"request_sha256": {name: digest(body) for name, body in bodies.items()},
                          "log_probs": scores, "answers": answers})
    return {"protocol_sha256": sha256(PROTOCOL.read_bytes()).hexdigest(), "cases": cases}


def field(answer):
    if answer is not None:
        numbers = re.findall(r"\b[0-9]+\b", answer)
        if numbers in (["30"], ["60"]):
            return numbers[0]
    return None


def reversals(log_probs):
    odds = {name: a - b for name, (a, b) in log_probs.items()}
    return [source for source in SOURCES if odds[source] * odds["baseline"] < 0]


def summarize(protocol, result):
    if result["protocol_sha256"] != sha256(PROTOCOL.read_bytes()).hexdigest():
        raise ValueError("Result file uses a different protocol.")
    selectors = {name: {"selected": 0, "resolved": 0, "switches": 0, "unresolved": 0}
                 for name in ("empty", "supplied", *SOURCES)}
    comparisons = {name: {key: 0 for key in ("tp", "fp", "tn", "fn", "unresolved")}
                   for name in ("empty", "supplied")}
    adherence = {name: {"completed": 0, "starts_with_prefix": 0} for name in ("baseline", *SOURCES)}
    any_change = 0
    for q, case in zip(protocol["questions"], result["cases"], strict=True):
        batch, _, _ = prepare(protocol, q)
        digests = {"baseline": batch["baseline_request_sha256"],
                   **{edit["source_id"]: edit["request_sha256"] for edit in batch["edits"]}}
        if digests != case["request_sha256"]:
            raise ValueError("Saved scores and answers use a different request or edit.")
        answers = case["answers"]
        fields = {name: field(answer) for name, answer in answers.items()}
        switched = {name: fields[name] != fields["baseline"] if fields[name] and fields["baseline"] else None
                    for name in SOURCES}
        any_change += True in switched.values()
        for name, answer in answers.items():
            if answer is not None:
                adherence[name]["completed"] += 1
                adherence[name]["starts_with_prefix"] += answer.startswith(q["prefix"])
        for name, counts in selectors.items():
            if name in SOURCES:
                selected = [name]
            else:
                selected = reversals(case["log_probs"][name])
                for source, changed in switched.items():
                    reversed_pair = source in selected
                    if changed is None:
                        key = "unresolved"
                    elif reversed_pair:
                        key = "tp" if changed else "fp"
                    else:
                        key = "fn" if changed else "tn"
                    comparisons[name][key] += 1
            if len(selected) == 1:
                changed = switched[selected[0]]
                counts["selected"] += 1
                counts["resolved"] += changed is not None
                counts["unresolved"] += changed is None
                counts["switches"] += changed is True
    return {"questions": len(result["cases"]), "any_switch": any_change, "selectors": selectors,
            "edit_comparisons": comparisons, "prefix_adherence": adherence}


def example(protocol, result, model, binary):
    q, case = protocol["questions"][0], result["cases"][0]
    _, request, edits = prepare(protocol, q)
    args = [model, "--model-id", protocol["checkpoint"]]
    test = {"version": 1, "id": "journal-supplied-prefix", "scorer_id": "qwen2-transformers-continuation-v1",
            "assistant_prefix": q["prefix"], "candidates": CANDIDATES, "interventions": edits}
    candidate = run_probe(binary, "probe-candidates", request, test,
                          [ROOT / "scripts/score_qwen_candidates.py", *args, "--revision", protocol["revision"], "--token-steps"])
    answers = case["answers"]
    trial = {"version": 1, "id": "journal-free-field", "runner_id": json.loads((HERE / protocol["reference"]).read_text())["runner_id"],
             "expected": {"kind": "model_output", "label": "Model answer", "outcome": answers["baseline"]},
             "samples": 1, "interventions": edits}
    shadow = run_probe(binary, "shadow", request, trial, [ROOT / "scripts/propose_qwen.py", *args])
    ids = {effect["condition_id"]: effect["source_id"] for effect in candidate["effects"]}
    ids["baseline"] = "baseline"
    actual = {ids[row["id"]]: row["log_probs"] for row in candidate["scores"]["conditions"]}
    if actual != case["log_probs"]["supplied"] or candidate["assistant_prefix"] != q["prefix"]:
        raise ValueError("The independently constructed candidate report differs from the study.")
    for report in (candidate, shadow):
        hashes = {"baseline": report["captured_request_sha256"],
                  **{e["source_id"]: e["request_sha256"] for e in report["effects"]}}
        if hashes != case["request_sha256"]:
            raise ValueError("The independent runner used different requests.")
    trials = {"baseline": shadow["baseline"], **{e["source_id"]: e["trials"] for e in shadow["effects"]}}
    actual = {name: rows[0].get("choice", {}).get("outcome") for name, rows in trials.items()}
    if actual != answers or any(len(rows) != 1 for rows in trials.values()):
        raise ValueError("The independently generated proposals differ from the study.")
    return {"request": request, "candidate": candidate, "shadow": shadow}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, help="Run the frozen protocol on this checkpoint.")
    parser.add_argument("--results", type=Path, default=HERE / "results.json")
    parser.add_argument("--example", type=Path, help="Independently generate the first question's dashboard report.")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/a2context")
    args = parser.parse_args()
    protocol = json.loads(PROTOCOL.read_text())
    if args.model and not args.example:
        if args.results.exists():
            parser.error("Choose a new results file.")
        save(args.results, measure(protocol, args.model))
    result = json.loads(args.results.read_text())
    print(json.dumps(summarize(protocol, result), indent=2))
    if args.example:
        if not args.model or args.example.exists():
            parser.error("A local checkpoint and a new output path are required for the example.")
        verify_model(args.model)
        save(args.example, example(protocol, result, args.model.resolve(), args.binary.resolve()))
