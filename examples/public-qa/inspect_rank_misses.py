"""Measure the first token where a lower-ranked edit changed a held-out greedy answer."""
import argparse
from hashlib import file_digest, sha256
import json
from pathlib import Path

from check_holdout import rank_review, verify_dataset
from check_passage_edits import dataset_records, first_step, js_bits, load_runtime
from view_passage_edits import body_digest, prepare


def measure(runtime, case, record, study):
    ranked = sorted(case["edits"], key=lambda e: -e["js_bits"])
    lower = next(e for e in ranked[1:] if e["answer"] is not None and e["answer"] != case["baseline"])
    _, _, conditions = prepare(record, case, study)
    prompts, generated, rows = [], [], []
    for label, edit in [("original", None), ("top", ranked[0]), ("lower", lower)]:
        key = (f"request-source:passage-{edit['position']}",) if edit else ()
        request, expected = conditions[key]
        prompt, ids, _, _ = runtime.prompt_input(request["messages"], None)
        answer, output = runtime.generate(prompt, ids, request)
        if answer != expected:
            raise ValueError(f"Row {case['row']} {label} no longer reproduces the saved answer.")
        prompts.append(ids)
        generated.append(output)
        rows.append({"condition":label, "passage":edit["position"] if edit else None,
                     "request_sha256":body_digest(request), "answer":answer, "answer_token_ids":output})

    original, top, changed = generated
    offset = 0
    while offset < min(len(original), len(changed)) and original[offset] == changed[offset]:
        offset += 1
    prefix = original[:offset]
    if top[:offset] != prefix:
        raise ValueError("The three generated paths do not share this prefix.")
    opening = [first_step(runtime, ids) for ids in prompts]
    scores = [first_step(runtime, ids + prefix) for ids in prompts] if prefix else opening
    leaders = [int(score.argmax().item()) for score in scores]
    for row, output, leader, score in zip(rows, generated, leaders, opening, strict=True):
        if offset < len(output):
            if leader != output[offset]:
                raise ValueError("The next-token readout disagrees with the generated token.")
        elif offset != len(output) or leader not in runtime.generation_config.eos_token_id:
            raise ValueError("Expected an observed end-of-sequence decision.")
        row["first_token_js_bits"] = js_bits(runtime.torch, opening[0], score)
    if leaders[0] == leaders[2]:
        raise ValueError("Expected different greedy tokens at the first differing position.")
    for row, leader, score in zip(rows, leaders, scores, strict=True):
        row["chosen_token_id"] = leader
        row["probability_original_token"] = score[leaders[0]].exp().item()
        row["probability_lower_token"] = score[leaders[2]].exp().item()
    for row, edit in zip(rows[1:], (ranked[0], lower), strict=True):
        if abs(row["first_token_js_bits"] - edit["js_bits"]) > 1e-8:
            raise ValueError("The first-token shift differs from the saved ranking.")
    return {"row":case["row"], "question_sha256":case["question_sha256"], "prefix_ids":prefix,
            "prefix":runtime.tokenizer.decode(prefix), "position":offset + 1,
            "original_token":runtime.tokenizer.decode([leaders[0]]), "lower_token":runtime.tokenizer.decode([leaders[2]]),
            "conditions":rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    study = json.loads(Path(__file__).with_name("holdout-results.json").read_text())
    verify_dataset(study, args.dataset)
    runtime = load_runtime(args.snapshot)
    if runtime.identity != study["runtime"] or sha256((args.snapshot / "tokenizer.json").read_bytes()).hexdigest() != study["tokenizer_sha256"]:
        raise ValueError("Use the runtime and tokenizer from the holdout.")
    result = {k:study[k] for k in ("cohort","rotation","dataset_sha256","model","revision","runtime","tokenizer_sha256")}
    with (args.snapshot / "model.safetensors").open("rb") as weights:
        result["weights_sha256"] = file_digest(weights, "sha256").hexdigest()
    result["stop_ids"] = runtime.generation_config.eos_token_id
    if result["weights_sha256"] != "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe":
        raise ValueError("Use the pinned Qwen2.5-0.5B-Instruct weights.")
    with runtime.torch.inference_mode():
        result["cases"] = [measure(runtime, case, record, study) for case, (_, record) in
                           zip(study["cases"], dataset_records(args.dataset, "holdout"), strict=True)
                           if rank_review(case) == "lower_changed"]
    with args.output.open("x") as handle:
        json.dump(result, handle, indent=2, allow_nan=False)
        handle.write("\n")


if __name__ == "__main__":
    main()
