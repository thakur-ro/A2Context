"""Check the two saved passage orders and count changes in the suggested test."""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
from statistics import median
from urllib.parse import urlencode
from urllib.request import urlopen

from check_passage_edits import summarize


def check_study(study):
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


def compare(first, second):
    for key in ("dataset_sha256", "model", "revision", "runtime", "tokenizer_sha256"):
        if first[key] != second[key]:
            raise ValueError(f"Different {key} across the two studies.")
    if (second["rotation"] - first["rotation"]) % 10 != 5:
        raise ValueError("Expected passage positions to differ by five slots.")
    for study in (first, second):
        check_study(study)

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


def check_dashboard(base, first, second):
    def arm(position, baseline, answer):
        return (position, *[s[:160] + "…" if len(s) > 160 else s for s in (baseline, answer)])

    expected = Counter()
    for a, b in zip(first["cases"], second["cases"], strict=True):
        rotated = {edit["passage_sha256"]: edit for edit in b["edits"]}
        for edit in a["edits"]:
            sha = edit["passage_sha256"]
            other = rotated[sha]
            if None in (edit["answer"], other["answer"]):
                continue
            left = arm(edit["position"], a["baseline"], edit["answer"])
            right = arm(other["position"], b["baseline"], other["answer"])
            changed = edit["answer"] != a["baseline"]
            if changed != (other["answer"] != b["baseline"]):
                expected[(sha, left, right) if changed else (sha, right, left)] += 1
    observed = Counter()
    offset = 0
    while offset is not None:
        query = urlencode({"org": "public-qa", "offset": offset})
        with urlopen(base.rstrip("/") + "/api/sidecar/replay-variations?" + query, timeout=15) as response:
            page = json.load(response)
        for row in page["rows"]:
            if not row["matched_fragment_order"]:
                raise ValueError("The dashboard could not verify an order-only request difference.")
            arms = [row[name] for name in ("changed_example", "equal_example")]
            observed[(arms[0]["sources"][0]["version"], *[(d["fragment_order"]["position"],
                      d["baseline_answer"], d["edited_answer"]) for d in arms])] += 1
        offset = page["next_offset"]
    if observed != expected:
        raise ValueError(f"Comparison mismatch: {sum((expected-observed).values())} missing, "
                         f"{sum((observed-expected).values())} unexpected.")
    return sum(observed.values())


if __name__ == "__main__":
    folder = Path(__file__).parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("first", nargs="?", type=Path, default=folder / "results.json")
    parser.add_argument("second", nargs="?", type=Path, default=folder / "rotated-results.json")
    parser.add_argument("--url", help="Also check the public-qa sidecar populated with both saved studies.")
    args = parser.parse_args()
    studies = [json.loads(path.read_text()) for path in (args.first, args.second)]
    print(json.dumps(compare(*studies), indent=2))
    if args.url:
        print(f"Verified {check_dashboard(args.url, *studies)} cross-request comparisons against the saved generations.")
    for study in studies:
        print(f"Rotation {study['rotation']}, median prefill time {median(study['scoring_seconds']):.2f} s")
        print(json.dumps(study["summary"], indent=2))
