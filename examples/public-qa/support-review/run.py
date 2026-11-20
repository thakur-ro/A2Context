"""Calibrate a forced-choice evidence check and score fixed pairs before generation."""
import argparse
import gzip
import json
import math
from pathlib import Path
import sys
from time import perf_counter

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from annotation_gap import DATA_SHA, WEIGHTS, digest, file_hash
from check_passage_edits import bm25, load_runtime, request
from review_packet import ROWS
from score_qwen_prefill import first_step

SYSTEM = ("Decide whether the supplied passages contain enough evidence to answer the question. "
          "Reply YES if the answer can be supported by the passages. Otherwise reply NO. Use only the passages.")
PILOT = range(0, 1000, 100)
PINNED = {
    "config.json": "18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45",
    "generation_config.json": "e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6",
    "tokenizer_config.json": "006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680",
    "model.safetensors": WEIGHTS,
    "tokenizer.json": "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
}
PREVIOUS_SHA = "23023b3555946cabd2de1e58a4ea7caaf04cbc29442b07d15b85f2e863b4479f"
PROTOCOL_SHA = "161f485184ead375e597ff80262f202f14e738338905c2f1f67484c6bdbc0233"


def inventories(record, index):
    gold, = [d for d in record["ctxs"] if d["isgold"]]
    reserved, *others = [d for d in record["ctxs"] if not d["isgold"]]
    if len(others) != 8:
        raise ValueError("Expected eight shared distractors.")
    slot = index % 9
    for arm, inserted in (("present", gold), ("missing", reserved)):
        docs = [*others[:slot], inserted, *others[slot:]]
        yield arm, [doc["title"] + "\n" + doc["text"] for doc in docs]


def no_probability(log_yes, log_no):
    high = max(log_yes, log_no)
    yes, no = math.exp(log_yes - high), math.exp(log_no - high)
    return no / (yes + no)


def support_score(runtime, messages, choices):
    start = perf_counter()
    _, ids, _, _ = runtime.prompt_input([{"role": "system", "content": SYSTEM}, messages[1]], None)
    logits = first_step(runtime, ids)
    yes, no = (logits[token].item() for token in choices)
    return {"score": no_probability(yes, no), "logp_yes": yes, "logp_no": no, "input_tokens": len(ids),
            "seconds": perf_counter() - start}


def abstention_score(runtime, messages, completion):
    start = perf_counter()
    _, ids, _, _ = runtime.prompt_input(messages, None)
    inputs = runtime.torch.tensor([ids + completion[:-1]])
    logits = runtime.model(input_ids=inputs, use_cache=False, logits_to_keep=len(completion)).logits[0]
    score = logits.double().log_softmax(dim=-1)[range(len(completion)), completion].sum().item()
    return {"score": score, "input_tokens": inputs.shape[1], "prompt_tokens": len(ids), "seconds": perf_counter() - start}


def lexical_score(question, texts):
    start = perf_counter()
    score = -max(bm25(question, texts))
    return {"score": score, "seconds": perf_counter() - start}


def measure(args):
    previous = HERE.parent / "annotation-gap-results.json"
    files = {args.dataset: DATA_SHA, previous: PREVIOUS_SHA, HERE / "protocol.md": PROTOCOL_SHA}
    files.update({args.snapshot / file: expected for file, expected in PINNED.items()})
    for path, expected in files.items():
        if file_hash(path) != expected:
            raise ValueError(f"Digest mismatch for {path.name}")

    prior = json.loads(previous.read_text())
    old_cases = json.loads((HERE.parent / "holdout-results.json").read_text())["cases"]
    previous_questions = {case["question_sha256"] for case in [*old_cases, *prior["pilot"], *prior["evaluation"]]}
    with gzip.open(args.dataset, "rt") as source:
        records = {i: json.loads(line) for i, line in enumerate(source) if i in PILOT or i in ROWS}
    if len(ROWS) != 12 or set(records) != set(PILOT) | set(ROWS):
        raise ValueError("The dataset must include all 12 evaluation and 10 calibration rows.")
    fresh = {digest(records[i]["question"]) for i in ROWS}
    if len(fresh) != 12 or fresh & previous_questions:
        raise ValueError("Evaluation questions must be unique and disjoint from prior samples.")

    runtime = load_runtime(args.snapshot)
    if runtime.identity != prior["runtime"]:
        raise ValueError("Runtime differs from the previous experiment.")
    (yes,), (no,) = [runtime.tokenizer.encode(word, add_special_tokens=False) for word in ("YES", "NO")]
    completion = runtime.tokenizer.encode("UNKNOWN", add_special_tokens=False)
    completion.append(runtime.tokenizer.convert_tokens_to_ids("<|im_end|>"))
    if completion != prior["completion_token_ids"]:
        raise ValueError("Abstention tokens differ from the previous experiment.")

    data = {key: prior[key] for key in ("dataset_sha256", "model", "revision", "runtime")}
    data.update(snapshot_sha256=PINNED, protocol_sha256=PROTOCOL_SHA, prior_scores_sha256=PREVIOUS_SHA,
                support_system=SYSTEM, choice_tokens={"YES": yes, "NO": no}, completion_tokens=completion)
    data["thresholds"] = {method: spec["threshold_strictly_above"] for method, spec in prior["summary"].items()}
    with runtime.torch.inference_mode():
        for split, rows in (("calibration", PILOT), ("evaluation", ROWS)):
            cases = []
            for index, row in enumerate(rows):
                record = records[row]
                question = record["question"]
                case = {"row": row, "question_sha256": digest(question), "slot": index % 9 + 1}
                for arm, texts in inventories(record, index):
                    if split == "calibration" and arm == "missing":
                        continue
                    messages = request(question, texts, None)
                    scores = {"passage_sha256": [digest(text) for text in texts],
                              "support_no": support_score(runtime, messages, (yes, no))}
                    if split == "evaluation":
                        scores["abstention_logp"] = abstention_score(runtime, messages, completion)
                        scores["negative_max_bm25"] = lexical_score(question, texts)
                    case[arm] = scores
                cases.append(case)
            data[split] = cases
            if split == "calibration":
                data["thresholds"]["support_no"] = sorted(c["present"]["support_no"]["score"] for c in cases)[-2]
    return data


def report(data):
    cases = data["evaluation"]
    counts, cost = {}, {}
    for method, threshold in data["thresholds"].items():
        pairs = [(c["present"][method], c["missing"][method]) for c in cases]
        scores = [(p["score"], m["score"]) for p, m in pairs]
        counts[method] = {"present_flagged": sum(p > threshold for p, m in scores),
                          "missing_flagged": sum(m > threshold for p, m in scores),
                          "missing_only_flagged": sum(m > threshold >= p for p, m in scores),
                          "missing_higher": sum(m > p for p, m in scores), "ties": sum(m == p for p, m in scores)}
        arms = [result for pair in pairs for result in pair]
        cost[method] = {"model_calls": len(arms) if method != "negative_max_bm25" else 0,
                        "input_tokens": sum(r.get("input_tokens", 0) for r in arms),
                        "prompt_tokens": sum(r.get("prompt_tokens", r.get("input_tokens", 0)) for r in arms),
                        "seconds": sum(r["seconds"] for r in arms)}
    cal = [c["present"]["support_no"] for c in data["calibration"]]
    cost["support_calibration"] = {"model_calls": len(cal), "input_tokens": sum(r["input_tokens"] for r in cal),
                                   "prompt_tokens": sum(r["input_tokens"] for r in cal),
                                   "seconds": sum(r["seconds"] for r in cal)}
    return {"source_swap": counts, "cost": cost}


def check(data):
    previous = HERE.parent / "annotation-gap-results.json"
    if file_hash(previous) != PREVIOUS_SHA or data["prior_scores_sha256"] != PREVIOUS_SHA:
        raise ValueError("The prior calibration file differs from the pinned run.")
    prior = json.loads(previous.read_text())
    if any(data["thresholds"][k] != v["threshold_strictly_above"] for k, v in prior["summary"].items()):
        raise ValueError("A frozen threshold changed.")
    if data["protocol_sha256"] != file_hash(HERE / "protocol.md") or data["snapshot_sha256"] != PINNED:
        raise ValueError("The saved provenance disagrees with the study files.")
    if [c["row"] for c in data["evaluation"]] != list(ROWS) or [c["row"] for c in data["calibration"]] != list(PILOT):
        raise ValueError("Unexpected question rows or order.")
    for saved, original in zip(data["calibration"], prior["pilot"]):
        hashes = saved["present"]["passage_sha256"]
        if (saved["question_sha256"] != original["question_sha256"]
                or hashes[saved["slot"] - 1] != original["present"]["replacement_sha256"]):
            raise ValueError("The calibration question or inserted passage changed.")
    for split in ("calibration", "evaluation"):
        for n, case in enumerate(data[split]):
            for arm in (("present",) if split == "calibration" else ("present", "missing")):
                item = case[arm]["support_no"]
                if no_probability(item["logp_yes"], item["logp_no"]) != item["score"]:
                    raise ValueError("Forced-choice probability differs from token probabilities.")
            if split == "evaluation":
                p, m = (case[arm]["passage_sha256"] for arm in ("present", "missing"))
                if len(p) != 9 or len(m) != 9 or [i for i in range(9) if p[i] != m[i]] != [n % 9]:
                    raise ValueError("Paired prompts must differ in the assigned slot only.")
    threshold = sorted(c["present"]["support_no"]["score"] for c in data["calibration"])[-2]
    if data["thresholds"]["support_no"] != threshold or report(data) != data["report"]:
        raise ValueError("Scores disagree with saved calibration, counts or cost.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", type=Path)
    parser.add_argument("--dataset", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.check:
        if any((args.dataset, args.snapshot, args.output)):
            parser.error("--check only accepts the saved file")
        data = json.loads(args.check.read_text())
        check(data)
        print(json.dumps(data["report"], indent=2))
        return
    if not all((args.dataset, args.snapshot, args.output)) or args.output.exists():
        parser.error("Provide --dataset, --snapshot and an unused --output path")
    data = measure(args)
    data["report"] = report(data)
    check(data)
    with args.output.open("x") as target:
        json.dump(data, target, indent=2, allow_nan=False)
        target.write("\n")


if __name__ == "__main__":
    main()
