"""Measure the frozen journal questions or reconstruct a pending inspection from saved scores."""
import argparse
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import sys

from test_target_tokens import run_probe

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2] / "scripts"))
from propose_qwen import propose
from score_qwen_prefill import load_runtime, score_batch

PROTOCOL = HERE / "question-holdout.json"
SOURCES = ("journal-policy", "journal-faq")
ATTRIBUTE = "a2context.prefill.probe"


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def digest(value):
    return sha256(compact(value).encode()).hexdigest()


def conditions(reference, question):
    body = deepcopy(reference["request"])
    body["messages"][-1]["content"] = question
    batch = {"version": 1, "baseline": body, "baseline_request_sha256": digest(body), "edits": []}
    declarations, edits = [], []
    for source, operation in zip(SOURCES, reference["interventions"], strict=True):
        edited = deepcopy(body)
        path = operation["pointer"]
        index = int(path.split("/")[2])
        assert path == f"/messages/{index}/content"
        original = body["messages"][index]["content"]
        edited["messages"][index]["content"] = operation["replacement"]
        field = {"pointer": path, "text_range": {"start_byte": 0, "end_byte": len(original.encode())}}
        declarations.append({"id": source, "source_id": source, "source_version": "1", "request_fragment": field})
        context_id = "request-source:" + source
        edits.append({"context_id":context_id, **field, "operation":"replace_text_fragment", "replacement":operation["replacement"]})
        batch["edits"].append({"context_id":context_id, "source_id":source, "source_version":"1",
                               "request_sha256":digest(edited), "request":edited})
    attributes = {"gen_ai.request.model":body["model"], "a2context.request.body":compact(body),
                  "a2context.context.semantics":{"version":1,"request_sources":declarations}}
    return batch, {"version":1, "attributes":attributes}, edits


def select(scores, historical):
    first, second = scores["effects"]
    ranked = sorted((first, second), key=lambda effect: -effect["js_bits"])
    flipped = [effect["source_id"] for effect in (first, second) if effect["top_token_id"] != scores["top_token_id"]]
    return {"history":historical, "first_listed":first["source_id"],
            "distribution":ranked[0]["source_id"] if ranked[0]["js_bits"] != ranked[1]["js_bits"] else None,
            "opening":flipped[0] if len(flipped) == 1 else None}


def verify_model(model):
    hasher = sha256()
    with (model / "model.safetensors").open("rb") as weights:
        for chunk in iter(lambda: weights.read(1024 * 1024), b""):
            hasher.update(chunk)
    if hasher.hexdigest() != "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe":
        raise ValueError("The model weights differ from the pinned journal checkpoint.")


def measure(model, protocol, reference):
    runtime = load_runtime(model)
    tokenizer_hash = sha256((model / "tokenizer.json").read_bytes()).hexdigest()
    cases = []
    with runtime.torch.inference_mode():
        for question in protocol["questions"]:
            batch, _, _ = conditions(reference, question)
            model_id = batch["baseline"]["model"]
            scores = score_batch(runtime, batch, model_id, protocol["model_revision"], tokenizer_hash, opening_tokens=True)
            choices = select(scores, protocol["historical_choice"])
            answers = {}
            for source, body in [("original",batch["baseline"]), *((e["source_id"],e["request"]) for e in batch["edits"])]:
                try:
                    answers[source] = propose(runtime, body, model_id)["outcome"]
                except ValueError as error:
                    if "generation limit" not in str(error):
                        raise
                    answers[source] = None
            cases.append({"question":question,"scores":scores,"selected":choices,"answers":answers})
    return {"protocol_sha256":sha256(PROTOCOL.read_bytes()).hexdigest(),"cases":cases}


def summarize(cases):
    selectors = {name:{"hits":0,"selections":0,"unresolved":0} for name in cases[0]["selected"]}
    openings = {result:{"changed":0,"held":0,"unresolved":0} for result in ("changed","held")}
    any_change = 0
    for case in cases:
        answers = case["answers"]
        outcomes = {s:"unresolved" if None in (answers[s],answers["original"]) else
                    "changed" if answers[s] != answers["original"] else "held" for s in SOURCES}
        any_change += "changed" in outcomes.values()
        for name, selected in case["selected"].items():
            if selected:
                counts = selectors[name]
                counts["selections"] += 1
                counts["hits"] += outcomes[selected] == "changed"
                counts["unresolved"] += outcomes[selected] == "unresolved"
        for effect in case["scores"]["effects"]:
            opening = "changed" if effect["top_token_id"] != case["scores"]["top_token_id"] else "held"
            openings[opening][outcomes[effect["source_id"]]] += 1
    return {"questions":len(cases),"any_verified_change":any_change,"selectors":selectors,"openings":openings}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_mutually_exclusive_group()
    commands.add_argument("--model", type=Path, help="Run all questions with this pinned local snapshot.")
    commands.add_argument("--preview", type=int, choices=range(1,9), help="Write a saved request with pre-generation scores.")
    parser.add_argument("--results", type=Path, default=HERE / "question-results.json")
    parser.add_argument("--verify-with", type=Path, metavar="MODEL", help="Independently rerun the displayed proposals using this local snapshot.")
    parser.add_argument("--binary", type=Path, default=HERE.parents[2] / "target/debug/a2context")
    args = parser.parse_args()
    if args.verify_with and not args.preview:
        parser.error("--verify-with needs a preview question.")
    protocol = json.loads(PROTOCOL.read_text())
    reference = json.loads((HERE / protocol["reference"]).read_text())
    if args.model or args.verify_with:
        verify_model(args.model or args.verify_with)
    if args.model:
        if args.results.exists():
            parser.error("Choose a new results file for this run.")
        study = measure(args.model, protocol, reference)
        with args.results.open("x") as output:
            output.write(json.dumps(study, indent=2, ensure_ascii=False, allow_nan=False) + "\n")
    else:
        study = json.loads(args.results.read_text())
    if study["protocol_sha256"] != sha256(PROTOCOL.read_bytes()).hexdigest() or [c["question"] for c in study["cases"]] != protocol["questions"]:
        raise ValueError("Results do not match the frozen protocol.")
    if args.preview:
        case = study["cases"][args.preview-1]
        _, request, edits = conditions(reference, case["question"])
        request["attributes"][ATTRIBUTE] = {"interventions":edits, "scores":case["scores"]}
        envelope = {"request":request}
        if args.verify_with:
            expected = case["answers"]["original"]
            if expected is None:
                parser.error("This question has no completed baseline to replicate.")
            test = {"version":1, "id":f"journal-opening-{args.preview}", "runner_id":reference["runner_id"],
                    "expected":{"kind":"model_output","label":"Model answer","outcome":expected},
                    "samples":1, "interventions":edits}
            report = run_probe(args.binary, "shadow", request, test,
                               [HERE.parents[2] / "scripts/propose_qwen.py", args.verify_with, "--model-id",case["scores"]["model_id"]])
            for source, trials in [("original",report["baseline"]), *((e["source_id"],e["trials"]) for e in report["effects"])]:
                if len(trials) != 1 or trials[0].get("choice", {}).get("outcome") != case["answers"][source]:
                    raise ValueError(f"The independent {source} proposal differs from the saved trial.")
            envelope["shadow"] = report
        print(json.dumps(envelope, ensure_ascii=False, indent=2))
    else:
        print(json.dumps(summarize(study["cases"]), indent=2))


if __name__ == "__main__":
    main()
