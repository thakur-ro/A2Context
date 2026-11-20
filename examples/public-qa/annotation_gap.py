"""Measure a candidate-only context-gap warning before generating a QA answer."""
import argparse
import gzip
from hashlib import sha256
import json
from pathlib import Path

from check_passage_edits import DATA_SHA, bm25, contains_reference, load_runtime, request

HERE = Path(__file__).resolve().parent
ROWS = {"pilot": range(0, 1000, 100), "evaluation": range(2000, 2500, 25)}
WEIGHTS = "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe"


def digest(text):
    return sha256(text.encode()).hexdigest()


def file_hash(path):
    hasher = sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def score(runtime, question, passages, completion):
    _, ids, _, _ = runtime.prompt_input(request(question, passages, None), None)
    inputs = runtime.torch.tensor([ids + completion[:-1]])
    logits = runtime.model(input_ids=inputs, use_cache=False, logits_to_keep=len(completion)).logits[0]
    terms = logits.double().log_softmax(dim=-1)[range(len(completion)), completion]
    return {"prompt_tokens": len(ids), "abstention_logp": terms.sum().item(),
            "negative_max_bm25": -max(bm25(question, passages))}


def evaluate(runtime, record, index, completion):
    docs = record["ctxs"]
    gold, = [doc for doc in docs if doc["isgold"]]
    reserve, *others = [doc for doc in docs if not doc["isgold"]]
    if len(others) != 8:
        raise ValueError("Expected one annotated passage and nine distractors.")
    slot = index % 9
    result = {"question_sha256": digest(record["question"]), "slot": slot + 1}
    for arm, replacement in (("present", gold), ("missing", reserve)):
        inventory = [*others[:slot], replacement, *others[slot:]]
        texts = [d["title"] + "\n" + d["text"] for d in inventory]
        result[arm] = {
            **score(runtime, record["question"], texts, completion),
            "replacement_sha256": digest(texts[slot]),
            "contains_alias": any(contains_reference(text, record["answers"]) for text in texts),
        }
    return result


def summarize(study):
    summary = {}
    for method in ("abstention_logp", "negative_max_bm25"):
        threshold = sorted((r["present"][method] for r in study["pilot"]), reverse=True)[1]
        rows = study["evaluation"]
        summary[method] = {
            "threshold_strictly_above": threshold,
            "missing_flagged": sum(r["missing"][method] > threshold for r in rows),
            "present_flagged": sum(r["present"][method] > threshold for r in rows),
            "missing_scored_higher": sum(r["missing"][method] > r["present"][method] for r in rows),
            "pairs_tied": sum(r["missing"][method] == r["present"][method] for r in rows),
        }
    return summary


def verify_saved(results):
    study = json.loads(results.read_text())
    seen = set()
    for split, expected in ROWS.items():
        cases = study[split]
        if [case["row"] for case in cases] != list(expected):
            raise ValueError(f"Unexpected rows in {split}")
        for index, case in enumerate(cases):
            question = case["question_sha256"]
            if question in seen or case["slot"] != index % 9 + 1:
                raise ValueError("Repeated question or unexpected replacement slot")
            if case["present"]["replacement_sha256"] == case["missing"]["replacement_sha256"]:
                raise ValueError("The paired conditions specify the same passage")
            seen.add(question)
    if summarize(study) != study["summary"]:
        raise ValueError("Saved thresholds or counts differ from the paired scores")
    print(json.dumps(study["summary"], indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path, nargs="?")
    parser.add_argument("snapshot", type=Path, nargs="?")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check-saved", action="store_true", help="Recompute the published counts without loading a model.")
    args = parser.parse_args()
    if args.check_saved:
        if any((args.dataset, args.snapshot, args.output)):
            parser.error("--check-saved takes no model, dataset or output file")
        verify_saved(HERE / "annotation-gap-results.json")
        return
    if not all((args.dataset, args.snapshot, args.output)):
        parser.error("Provide a dataset, a model snapshot and --output")
    if args.output.exists():
        parser.error("Choose a new output file.")
    prior = json.loads((HERE / "holdout-results.json").read_text())
    for path, expected in ((args.dataset, DATA_SHA), (args.snapshot / "model.safetensors", WEIGHTS),
                           (args.snapshot / "tokenizer.json", prior["tokenizer_sha256"])):
        if file_hash(path) != expected:
            raise ValueError(f"Digest differs from the pinned file: {path.name}")
    with gzip.open(args.dataset, "rt") as stream:
        records = {i: json.loads(line) for i, line in enumerate(stream) if any(i in rows for rows in ROWS.values())}
    runtime = load_runtime(args.snapshot)
    if runtime.identity != prior["runtime"]:
        raise ValueError("Use the pinned runtime from the holdout experiment.")
    stop = runtime.tokenizer.convert_tokens_to_ids("<|im_end|>")
    if stop not in runtime.generation_config.eos_token_id:
        raise ValueError("Assistant stop token disagrees with the generation config.")
    completion = runtime.tokenizer.encode("UNKNOWN", add_special_tokens=False) + [stop]
    result = {key: prior[key] for key in ("dataset_sha256", "model", "revision", "runtime", "tokenizer_sha256")}
    result.update(weights_sha256=WEIGHTS, completion_token_ids=completion)
    seen = {c["question_sha256"] for c in prior["cases"]}
    with runtime.torch.inference_mode():
        for split, rows in ROWS.items():
            result[split] = []
            for index, row in enumerate(rows):
                case = evaluate(runtime, records[row], index, completion)
                if case["question_sha256"] in seen:
                    raise ValueError("A question repeats within or across splits.")
                seen.add(case["question_sha256"])
                result[split].append({"row": row, **case})
    result["summary"] = summarize(result)
    with args.output.open("x") as out:
        json.dump(result, out, indent=2, allow_nan=False)
        out.write("\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
