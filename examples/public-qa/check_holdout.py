"""Check the fixed cohort, rerank the measured edits and reproduce the paired comparison."""
import argparse
from collections import Counter
from hashlib import sha256
import json
from math import comb
from pathlib import Path
from statistics import median

from check_passage_edits import COHORTS, DATA_SHA, contains_reference, dataset_records, ordered_documents
from compare_orders import check_study


def verify_dataset(study, path):
    for case, (row, record) in zip(study["cases"], dataset_records(path, "holdout"), strict=True):
        if case["row"] != row or sha256(record["question"].encode()).hexdigest() != case["question_sha256"]:
            raise ValueError("Question identity differs from the pinned dataset.")
        for edit, doc in zip(case["edits"], ordered_documents(row, record), strict=True):
            content = doc["title"] + "\n" + doc["text"]
            if (edit["passage_sha256"], edit["is_gold"], edit["retrieval_index"]) != (
                sha256(content.encode()).hexdigest(), doc["isgold"], doc.get("original_retrieval_index")
            ):
                raise ValueError("Edited passage differs from the pinned dataset.")
        for answer, flag in [(case["baseline"], case["baseline_alias_match"]),
                             *((e["answer"], e["alias_match"]) for e in case["edits"])]:
            expected = None if answer is None else contains_reference(answer, record["answers"])
            if flag != expected:
                raise ValueError("Reference-alias result differs from the saved answer.")


def compare(held, pilot):
    if held["cohort"] != "holdout" or held["dataset_sha256"] != DATA_SHA or held["rotation"] != 0:
        raise ValueError("Expected the fixed holdout on the pinned dataset at rotation zero.")
    for key in ("dataset_sha256", "model", "revision", "runtime", "tokenizer_sha256"):
        if held[key] != pilot[key]:
            raise ValueError(f"Holdout and pilot differ in {key}.")
    cases = held["cases"]
    if [c["row"] for c in cases] != list(COHORTS["holdout"]):
        raise ValueError("Holdout does not contain every specified row in order.")
    questions = {c["question_sha256"] for c in cases}
    if len(questions) != len(cases) or questions.intersection(c["question_sha256"] for c in pilot["cases"]):
        raise ValueError("Questions repeat within the holdout or overlap the pilot.")
    check_study(held)
    paired = Counter(ranked_only=0, first_only=0, both=0, neither=0, missing=0)
    same_passage = 0
    for case in cases:
        baseline = case["baseline"]
        ranked = case["edits"][case["top_js"]-1]["answer"]
        first = case["edits"][0]["answer"]
        same_passage += case["top_js"] == 1
        if None in (baseline, ranked, first):
            paired["missing"] += 1
        elif (ranked != baseline) == (first != baseline):
            paired["both" if ranked != baseline else "neither"] += 1
        else:
            paired["ranked_only" if ranked != baseline else "first_only"] += 1
    scored = len(cases) - paired["missing"]
    wins, losses = paired["ranked_only"], paired["first_only"]
    discordant = wins + losses
    tail = sum(comb(discordant, k) for k in range(min(wins, losses)+1)) / 2**discordant
    return {"questions": len(cases), "paired": dict(paired), "same_selected_passage": same_passage,
            "scored_pairs": scored, "difference_pp": 100*(wins-losses)/scored if scored else None,
            "two_sided_exact_p": min(1, 2*tail) if scored else None,
            "median_ranking_seconds": median(held["scoring_seconds"]), "selectors": held["summary"]}


if __name__ == "__main__":
    folder = Path(__file__).parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path, nargs="?", default=folder / "holdout-results.json")
    parser.add_argument("--dataset", type=Path, help="Also verify passage identity, order and alias matching.")
    args = parser.parse_args()
    held = json.loads(args.results.read_text())
    result = compare(held, json.loads((folder / "results.json").read_text()))
    if args.dataset:
        verify_dataset(held, args.dataset)
    print(json.dumps(result, indent=2))
