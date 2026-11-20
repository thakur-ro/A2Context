"""Join frozen evidence annotations to the exact passage inventories that were scored."""
from collections import Counter
from hashlib import sha256
import argparse
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
LABELS_SHA256 = "f6dbc7134808f79966a131780e8bb4988dd991663e23829d2b059de410aeb1a4"


def classify(label, inventory):
    for category in ("reference", "other_answer", "uncertain"):
        if any(set(group) <= inventory for group in label[category]):
            return category
    return "no_support"


def compare(labels, scores):
    by_row = {label["row"]: label for label in labels}
    counts = {method: Counter() for method in scores["thresholds"]}
    cases = []
    for case in scores["evaluation"]:
        label = by_row.pop(case["row"])
        arms = {}
        for name in ("present", "missing"):
            hashes = case[name]["passage_sha256"]
            if len(hashes) != 9 or len(set(hashes)) != 9:
                raise ValueError("Expected nine distinct passages in each request")
            arms[name] = set(hashes)
        pool = arms["present"] | arms["missing"]
        if (len(pool) != 10 or
                sha256("\n".join(sorted(pool)).encode()).hexdigest() != label["inventory_sha256"] or
                case["question_sha256"] != label["question_sha256"]):
            raise ValueError("The scored and reviewed inventories differ")
        for category in ("reference", "other_answer", "uncertain"):
            if any(not group or not set(group) <= pool for group in label[category]):
                raise ValueError("A witness contains an unknown passage or an empty clause")
        summary = {"row": case["row"]}
        for name, inventory in arms.items():
            category = classify(label, inventory)
            alerts = []
            for method, threshold in scores["thresholds"].items():
                alert = case[name][method]["score"] > threshold
                if alert:
                    alerts.append(method)
                if category == "uncertain":
                    outcome = "uncertain_alert" if alert else "uncertain_quiet"
                elif category == "no_support":
                    outcome = "true_alert" if alert else "miss"
                else:
                    outcome = "false_alert" if alert else "supported_quiet"
                counts[method][outcome] += 1
            summary[name] = {"support": category, "alerts": alerts}
        cases.append(summary)
    if by_row:
        raise ValueError("Some reviewed questions were never scored")
    return {"counts": counts, "cases": cases}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scores", type=Path, default=HERE / "scores.json")
    args = parser.parse_args()
    raw = (HERE / "labels.json").read_bytes()
    if sha256(raw).hexdigest() != LABELS_SHA256:
        raise ValueError("The score-blind review has changed")
    print(json.dumps(compare(json.loads(raw), json.loads(args.scores.read_text())), indent=2))
