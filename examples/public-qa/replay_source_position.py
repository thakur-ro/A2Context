"""Replay one recommended passage edit, moving only that passage among its neighbors."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

from check_passage_edits import dataset_records, load_runtime, ordered_documents, request


def trial(runtime, question, passages, removed):
    prompt, ids, *_ = runtime.prompt_input(request(question, passages, removed), None)
    try:
        answer, _ = runtime.generate(prompt, ids, {"temperature": 0, "max_tokens": 48})
    except ValueError as error:
        if "generation limit" not in str(error):
            raise
        return None
    return answer


def measure(runtime, record, studies, row, target):
    cases = [next(case for case in study["cases"] if case["row"] == row) for study in studies]
    if any(sum(edit["passage_sha256"] == target for edit in case["edits"]) != 1 for case in cases):
        raise ValueError("Choose a source digest occurring exactly once in each order.")
    slots = [next(edit["position"] for edit in case["edits"] if edit["passage_sha256"] == target) for case in cases]
    if slots[0] == slots[1]:
        raise ValueError("The passage must occupy two different slots.")
    conditions = []
    for study, case, original_slot in zip(studies, cases, slots, strict=True):
        docs = ordered_documents(row, record, study["rotation"])
        passages = [d["title"] + "\n" + d["text"] for d in docs]
        hashes = [sha256(text.encode()).hexdigest() for text in passages]
        if hashes != [edit["passage_sha256"] for edit in case["edits"]]:
            raise ValueError("Dataset passages differ from the saved measurements.")
        passage = passages.pop(original_slot - 1)
        hashes.pop(original_slot - 1)
        results = []
        for slot in slots:
            inputs = passages.copy()
            inputs.insert(slot - 1, passage)
            baseline = trial(runtime, record["question"], inputs, None)
            edited = trial(runtime, record["question"], inputs, slot - 1)
            if slot == original_slot and (baseline, edited) != (case["baseline"], case["edits"][slot - 1]["answer"]):
                raise ValueError("The original replay did not reproduce the saved answers.")
            results.append({"slot": slot, "baseline": baseline, "edited": edited})
        conditions.append({"background_rotation": study["rotation"], "other_passages": hashes, "trials": results})
    return {"row": row, "question_sha256": cases[0]["question_sha256"],
            "source_sha256": target, "conditions": conditions}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--row", type=int, required=True)
    parser.add_argument("--source-sha256", required=True, help="Source version shown by the public QA loader.")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    studies = [json.loads(Path(__file__).with_name(name).read_text()) for name in ("results.json", "rotated-results.json")]
    config = {key: studies[0][key] for key in ("dataset_sha256", "model", "revision", "runtime", "tokenizer_sha256")}
    if any(studies[1][key] != value for key, value in config.items()):
        parser.error("The saved studies used different runner or dataset settings.")
    record = dict(dataset_records(args.dataset)).get(args.row)
    if record is None:
        parser.error("Choose one of the saved public QA rows.")
    tokenizer = sha256((args.snapshot / "tokenizer.json").read_bytes()).hexdigest()
    weights = sha256()
    with (args.snapshot / "model.safetensors").open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            weights.update(chunk)
    if weights.hexdigest() != "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe":
        parser.error("Expected the pinned Qwen2.5-0.5B-Instruct weights used for this study.")
    config["weights_sha256"] = weights.hexdigest()
    runtime = load_runtime(args.snapshot)
    if tokenizer != config["tokenizer_sha256"] or runtime.identity != config["runtime"]:
        parser.error("The local tokenizer or runtime differs from the saved studies.")
    with runtime.torch.inference_mode():
        result = measure(runtime, record, studies, args.row, args.source_sha256)
    with args.output.open("x") as handle:
        json.dump({**config, **result}, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
