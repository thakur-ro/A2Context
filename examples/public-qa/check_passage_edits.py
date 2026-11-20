"""Rank ten public QA passages by first-step Jensen–Shannon divergence and check greedy answers."""
import argparse
from collections import Counter
import gzip
from hashlib import sha256
import json
import math
from pathlib import Path
import re
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from probe_transformers_answer_timeline import load_runtime

DATA_SHA = "f8425253dafc19cd9de31c06ab6c8b57d4da2bf48b8ea2d0790b805fb3aa1968"
COHORTS = {"pilot": range(0, 1000, 100), "holdout": range(1000, 2000, 50)}
SYSTEM = "Answer the question using the passages. Give only the short answer. If the passages do not give an answer, reply UNKNOWN."


def tokens(text):
    return re.findall(r"\w+", text.casefold())


def bm25(question, passages):
    docs = [Counter(tokens(p)) for p in passages]
    lengths = [sum(d.values()) for d in docs]
    mean = sum(lengths) / len(docs)
    scores = [0.0] * len(docs)
    for term in sorted(set(tokens(question))):
        df = sum(term in d for d in docs)
        idf = math.log1p((len(docs) - df + 0.5) / (df + 0.5))
        for i, (doc, length) in enumerate(zip(docs, lengths)):
            tf = doc[term]
            scores[i] += idf * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * length / mean))
    return scores


def request(question, passages, removed):
    inventory = "\n\n".join(
        f"[{i + 1}] " + (text if i != removed else "[passage omitted]")
        for i, text in enumerate(passages)
    )
    return [{"role": "system", "content": SYSTEM}, {"role": "user", "content": inventory + "\n\nQuestion: " + question}]


def first_step(runtime, ids):
    logits = runtime.model(input_ids=runtime.torch.tensor([ids]), use_cache=False, logits_to_keep=1).logits[0, -1]
    return logits.double().log_softmax(dim=0)


def js_bits(torch, p, q):
    log_mean = torch.logaddexp(p, q) - math.log(2)
    return ((p.exp() * (p - log_mean)).sum() + (q.exp() * (q - log_mean)).sum()).item() / (2 * math.log(2))


def contains_reference(answer, aliases):
    # Token boundaries avoid a match such as "four" in "fourteen".
    normalized = " " + " ".join(tokens(answer)) + " "
    return any(" " + " ".join(tokens(a)) + " " in normalized for a in aliases if tokens(a))


def ordered_documents(row, record, rotation=0):
    documents = record["ctxs"]
    cut = (row // 100 + rotation) % len(documents)
    documents = documents[cut:] + documents[:cut]
    if len(documents) != 10:
        raise ValueError("Each selected row must have ten passages.")
    return documents


def evaluate(runtime, row, record, rotation):
    documents = ordered_documents(row, record, rotation)
    question = record["question"]
    passages = [d["title"] + "\n" + d["text"] for d in documents]
    conditions = [runtime.prompt_input(request(question, passages, i), None)[:2]
                  for i in [None, *range(len(passages))]]
    start = time.perf_counter()
    baseline = first_step(runtime, conditions[0][1])
    scores = []
    for _, ids in conditions[1:]:
        edited = first_step(runtime, ids)
        scores.append(js_bits(runtime.torch, baseline, edited))
    score_seconds = time.perf_counter() - start
    # Rank the passages before decoding the baseline or any variant.
    ranked = sorted(range(len(passages)), key=lambda i: (-scores[i], i))
    keyword = bm25(question, passages)
    lexical = max(range(len(passages)), key=keyword.__getitem__)
    answers = []
    for prompt, ids in conditions:
        try:
            answer, _ = runtime.generate(prompt, ids, {"temperature": 0, "max_tokens": 48})
        except ValueError as error:
            if "generation limit" not in str(error):
                raise
            answer = None
        answers.append(answer)
    if sum(doc["isgold"] for doc in documents) != 1:
        raise ValueError("Each evaluated question must have one annotated passage.")
    data = []
    for i, doc in enumerate(documents):
        answer = answers[i + 1]
        data.append({"position": i + 1, "passage_sha256": sha256(passages[i].encode()).hexdigest(),
                     "is_gold": doc["isgold"], "retrieval_index": doc.get("original_retrieval_index"),
                     "js_bits": scores[i], "bm25": keyword[i], "answer": answer,
                     "alias_match": None if answer is None else contains_reference(answer, record["answers"])})
    return {"row": row, "question_sha256": sha256(question.encode()).hexdigest(), "prompt_tokens": len(conditions[0][1]),
            "top_js": ranked[0] + 1, "top_bm25": lexical + 1, "baseline": answers[0],
            "baseline_alias_match": None if answers[0] is None else contains_reference(answers[0], record["answers"]),
            "edits": data}, score_seconds


def summarize(cases):
    def counts(pairs):
        complete = [(case, edit) for case, edit in pairs
                    if case["baseline"] is not None and edit["answer"] is not None]
        matched = [(case, edit) for case, edit in complete if case["baseline_alias_match"]]
        return {
            "tests": len(pairs),
            "gold": sum(edit["is_gold"] for _, edit in pairs),
            "scored": len(complete),
            "changed_text": sum(case["baseline"] != edit["answer"] for case, edit in complete),
            "baseline_alias_matches": len(matched),
            "lost_alias_match": sum(not edit["alias_match"] for _, edit in matched),
        }

    result = {}
    for key in ("top_js", "top_bm25"):
        result[key] = counts([(case, case["edits"][case[key] - 1]) for case in cases])
    result["first_position"] = counts([(case, case["edits"][0]) for case in cases])
    result["last_position"] = counts([(case, case["edits"][-1]) for case in cases])
    retrieved = []
    for case in cases:
        ranked = [edit for edit in case["edits"] if edit["retrieval_index"] is not None]
        retrieved.append((case, min(ranked, key=lambda edit: edit["retrieval_index"])))
    result["first_retrieved"] = counts(retrieved)
    result["all_edits"] = counts([(case, edit) for case in cases for edit in case["edits"]])
    return result


def dataset_records(path, cohort="pilot"):
    selected = COHORTS[cohort]
    if sha256(path.read_bytes()).hexdigest() != DATA_SHA:
        raise ValueError("Expected the pinned Lost in the Middle gold-at-4 ten-document dataset.")
    with gzip.open(path, "rt") as rows:
        for row, line in enumerate(rows):
            if row > selected[-1]:
                break
            if row in selected:
                yield row, json.loads(line)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--cohort", choices=COHORTS, default="pilot")
    parser.add_argument("--rotation", type=int, choices=range(10), default=0)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    runtime = load_runtime(args.snapshot)
    cases = []
    seconds = []
    with runtime.torch.inference_mode():
        for row, record in dataset_records(args.dataset, args.cohort):
            case, elapsed = evaluate(runtime, row, record, args.rotation)
            cases.append(case)
            seconds.append(elapsed)
            print(f"Checked public row {row}.", file=sys.stderr, flush=True)
    assert len(cases) == len(COHORTS[args.cohort])
    result = {
        "cohort": args.cohort, "scoring_seconds": seconds, "finished_unix_ns": time.time_ns(), "rotation": args.rotation,
        "dataset_sha256": DATA_SHA, "model": args.model_id, "revision": args.revision,
        "runtime": runtime.identity, "tokenizer_sha256": sha256((args.snapshot / "tokenizer.json").read_bytes()).hexdigest(),
        "cases": cases, "summary": summarize(cases),
    }
    with args.output.open("x") as handle:
        json.dump(result, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    print(json.dumps({"summary":result["summary"], "scoring_seconds": seconds}, indent=2))


if __name__ == "__main__":
    main()
