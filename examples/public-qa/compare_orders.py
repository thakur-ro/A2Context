"""Check the two saved passage orders and count changes in the suggested test."""
import argparse
import json
import math
from pathlib import Path
from statistics import median

from check_passage_edits import summarize


def compare(first, second):
    for key in ("dataset_sha256", "model", "revision", "runtime", "tokenizer_sha256"):
        if first[key] != second[key]:
            raise ValueError(f"Different {key} across the two studies.")
    if (second["rotation"] - first["rotation"]) % 10 != 5:
        raise ValueError("Expected passage positions to differ by five slots.")
    for study in (first, second):
        for case in study["cases"]:
            edits = case["edits"]
            if [e["position"] for e in edits] != list(range(1, 11)):
                raise ValueError("Expected ten numbered passage edits.")
            if any(not math.isfinite(e["js_bits"]) or not 0 <= e["js_bits"] <= 1 for e in edits):
                raise ValueError("Expected finite Jensen–Shannon divergences between zero and one bit.")
            for ranking, score in (("top_js", "js_bits"), ("top_bm25", "bm25")):
                if max(edits, key=lambda e: e[score])["position"] != case[ranking]:
                    raise ValueError(f"Saved {ranking} differs from its passage scores.")
        if summarize(study["cases"]) != study["summary"]:
            raise ValueError("Saved summary differs from the individual trials.")

    selected_source = selected_position = changed_baseline = 0
    for before, after in zip(first["cases"], second["cases"], strict=True):
        if (before["row"], before["question_sha256"]) != (after["row"], after["question_sha256"]):
            raise ValueError("Different questions across the two studies.")
        sources = [edit["passage_sha256"] for edit in before["edits"]]
        if sources[5:] + sources[:5] != [edit["passage_sha256"] for edit in after["edits"]]:
            raise ValueError("Passages differ beyond the five-slot rotation.")
        earlier = before["edits"][before["top_js"] - 1]
        later = after["edits"][after["top_js"] - 1]
        selected_source += earlier["passage_sha256"] == later["passage_sha256"]
        selected_position += earlier["position"] == later["position"]
        if before["baseline"] is None or after["baseline"] is None:
            raise ValueError("The paired baseline comparison needs two complete generations.")
        changed_baseline += before["baseline"] != after["baseline"]
    return {"questions": len(first["cases"]), "same_top_source": selected_source,
            "same_top_position": selected_position, "changed_baseline": changed_baseline}


if __name__ == "__main__":
    folder = Path(__file__).parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("first", nargs="?", type=Path, default=folder / "results.json")
    parser.add_argument("second", nargs="?", type=Path, default=folder / "rotated-results.json")
    args = parser.parse_args()
    studies = [json.loads(path.read_text()) for path in (args.first, args.second)]
    print(json.dumps(compare(*studies), indent=2))
    for study in studies:
        print(f"Rotation {study['rotation']}, median prefill time {median(study['scoring_seconds']):.2f} s")
        print(json.dumps(study["summary"], indent=2))
