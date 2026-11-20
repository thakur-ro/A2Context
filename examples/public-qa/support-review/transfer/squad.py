"""Apply the frozen support alarms to paired SQuAD 2.0 development questions."""
import argparse
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from run import (PINNED, abstention_score, digest, file_hash, lexical_score, load_runtime,
                 no_probability, request, support_score)

DATA_SHA = "80a5225e94905956a6446d296ca1093975c4d3b3260f1d6c8f68bc2ab77182d8"
CALIBRATION_SHA = "bf15081c3e7bf0f9667cad3a6335f45fe5f8383c853ffd7f011a1df71caf24ce"
PROTOCOL_SHA = "ec25685bd3d683a810a8605890593d8d618a93d328e34fbc26ac7fc0c9c3c9a7"


def select(dataset):
    if file_hash(dataset) != DATA_SHA:
        raise ValueError("Expected the official pinned development file.")
    titles = set()
    for article in json.loads(dataset.read_text())["data"]:
        if article["title"] in titles:
            continue
        for index, paragraph in enumerate(article["paragraphs"]):
            questions = {}
            for q in paragraph["qas"]:
                questions.setdefault(q["is_impossible"], q)
            if set(questions) != {True, False}:
                continue
            titles.add(article["title"])
            for label in (False, True):
                q = questions[label]
                fields = {"article_sha256": digest(article["title"]), "paragraph_index": index,
                          "context_sha256": digest(paragraph["context"]), "id": q["id"],
                          "question_sha256": digest(q["question"]), "is_impossible": label}
                yield fields, q["question"], paragraph["context"]
            break
        if len(titles) == 24:
            return
    raise ValueError("Fewer than 24 eligible articles.")


def calibration():
    path = HERE.parent / "scores.json"
    if file_hash(path) != CALIBRATION_SHA or file_hash(HERE / "protocol.md") != PROTOCOL_SHA:
        raise ValueError("The frozen thresholds or selection protocol changed.")
    return json.loads(path.read_text())


def summarize(cases, thresholds):
    if len(cases) != 48 or len({c["id"] for c in cases}) != 48 or len({c["article_sha256"] for c in cases}) != 24:
        raise ValueError("Expected 48 distinct questions from 24 articles.")
    for allowed, impossible in zip(cases[::2], cases[1::2]):
        if (allowed["is_impossible"] or not impossible["is_impossible"]
                or (allowed["article_sha256"], allowed["context_sha256"]) !=
                   (impossible["article_sha256"], impossible["context_sha256"])):
            raise ValueError("Expected one question per class about the same article and paragraph.")
    summaries = {}
    for method, threshold in thresholds.items():
        confusion = {"tp": 0, "fp": 0, "fn": 0, "tn": 0}
        measurements = [c[method] for c in cases]
        for c, obs in zip(cases, measurements):
            alarm = obs["score"] > threshold
            if c["is_impossible"]:
                confusion["tp" if alarm else "fn"] += 1
            else:
                confusion["fp" if alarm else "tn"] += 1
        delta = [b["score"] - a["score"] for a, b in zip(measurements[::2], measurements[1::2])]
        summaries[method] = {**confusion, "sensitivity": confusion["tp"] / 24, "specificity": confusion["tn"] / 24,
                             "unanswerable_higher": sum(x > 0 for x in delta), "ties": delta.count(0),
                             "model_calls": sum("input_tokens" in r for r in measurements),
                             "input_tokens": sum(r.get("input_tokens", 0) for r in measurements),
                             "prompt_tokens": sum(r.get("prompt_tokens", r.get("input_tokens", 0)) for r in measurements),
                             "seconds": sum(r["seconds"] for r in measurements)}
    return summaries


def measure(dataset, snapshot, frozen):
    for name, expected in PINNED.items():
        if file_hash(snapshot / name) != expected:
            raise ValueError(f"Snapshot differs for {name}")
    rt = load_runtime(snapshot)
    if rt.identity != frozen["runtime"]:
        raise ValueError("Runtime differs from the calibration run.")
    cases = []
    choices = [frozen["choice_tokens"][w] for w in ("YES", "NO")]
    with rt.torch.inference_mode():
        for c, question, paragraph in select(dataset):
            messages = request(question, [paragraph], None)
            cases.append({**c, "support_no": support_score(rt, messages, choices),
                          "abstention_logp": abstention_score(rt, messages, frozen["completion_tokens"]),
                          "negative_max_bm25": lexical_score(question, [paragraph])})
    data = {k: frozen[k] for k in ("model", "revision", "runtime", "snapshot_sha256", "thresholds")}
    data.update(dataset_sha256=DATA_SHA, calibration_sha256=CALIBRATION_SHA, protocol_sha256=PROTOCOL_SHA,
                cases=cases, summary=summarize(cases, frozen["thresholds"]))
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    frozen = calibration()
    if args.check:
        if args.snapshot or args.output:
            parser.error("--check takes a saved file and an optional --dataset")
        data = json.loads(args.check.read_text())
        cases = data["cases"]
        if (data["dataset_sha256"] != DATA_SHA or data["thresholds"] != frozen["thresholds"]
                or data["protocol_sha256"] != PROTOCOL_SHA or data["calibration_sha256"] != CALIBRATION_SHA
                or summarize(cases, data["thresholds"]) != data["summary"]):
            raise ValueError("Saved counts or provenance differ from the frozen design.")
        for c in cases:
            support = c["support_no"]
            if support["score"] != no_probability(support["logp_yes"], support["logp_no"]):
                raise ValueError("Stored probabilities disagree with forced-choice scores.")
        if args.dataset:
            manifest = [c for c, question, paragraph in select(args.dataset)]
            if [{k: case[k] for k in row} for case, row in zip(cases, manifest)] != manifest:
                raise ValueError("Question context, labels or selection differ from the official dataset.")
        print(json.dumps(data["summary"], indent=2))
        return
    if not all((args.dataset, args.snapshot, args.output)) or args.output.exists():
        parser.error("Supply --dataset, --snapshot and an unused --output path.")
    data = measure(args.dataset, args.snapshot, frozen)
    with args.output.open("x") as out:
        json.dump(data, out, indent=2, allow_nan=False)
        out.write("\n")


if __name__ == "__main__":
    main()
