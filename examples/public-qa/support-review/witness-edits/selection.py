"""Preselect reviewed evidence edits and equal-size controls without loading a model."""
import argparse
import gzip
from itertools import combinations, permutations
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from compare import LABELS_SHA256
from run import DATA_SHA, ROWS, digest, file_hash, inventories

SCORES_SHA = "bf15081c3e7bf0f9667cad3a6335f45fe5f8383c853ffd7f011a1df71caf24ce"


def select(texts, label):
    hashes = [digest(t) for t in texts]
    available = set(hashes)
    sufficient = [set(group) for group in label["reference"] + label["other_answer"] if set(group) <= available]
    if not sufficient:
        return {"excluded": "no complete sufficient set"}
    reserved = {p for category in ("reference", "other_answer", "uncertain") for group in label[category] for p in group}
    controls = [i for i, h in enumerate(hashes, 1) if h not in reserved]
    candidates = [i for i, h in enumerate(hashes, 1) if any(h in group for group in sufficient)]
    lengths = {i: len(t.split()) for i, t in enumerate(texts, 1)}
    for k in range(1, len(candidates) + 1):
        targets = [t for t in combinations(candidates, k) if
                   all(any(hashes[i - 1] in group for i in t) for group in sufficient)]
        if targets:
            break
    if k > len(controls):
        return {"excluded": "too few unreviewed control slots", "minimum_edits": k, "eligible_controls": controls}
    matches = sorted((sum(abs(lengths[i] - lengths[j]) for i, j in zip(t, c)), sum(abs(i - j) for i,j in zip(t,c)), t, c)
                     for t in targets for c in permutations(controls, k))
    word_gap, slot_gap, target, control = matches[0]
    return {"target": list(target), "control": list(control), "sum_absolute_word_gap": word_gap,
            "sum_absolute_slot_gap": slot_gap, "tied_assignments": sum(x[:2] == matches[0][:2] for x in matches)}


def records(path):
    if file_hash(path) != DATA_SHA:
        raise ValueError("Input differs from the pinned ten-document file.")
    with gzip.open(path, "rt") as f:
        return {i: json.loads(line) for i,line in enumerate(f) if i in ROWS}


def manifest(rows):
    labels_path, scores_path = HERE.parent / "labels.json", HERE.parent / "scores.json"
    if file_hash(labels_path) != LABELS_SHA256 or file_hash(scores_path) != SCORES_SHA:
        raise ValueError("Scored inventories or frozen review changed.")
    labels = {c["row"]: c for c in json.loads(labels_path.read_text())}
    prior = json.loads(scores_path.read_text())["evaluation"]
    cases = []
    for index, row in enumerate(ROWS):
        r, label = rows[row], labels[row]
        _, texts = next(inventories(r, index))
        if ([digest(t) for t in texts] != prior[index]["present"]["passage_sha256"]
                or row != prior[index]["row"] or digest(r["question"]) != label["question_sha256"]):
            raise ValueError("The input inventory differs from the previous experiment.")
        cases.append({"row": row, **select(texts, label)})
    return {"dataset_sha256": DATA_SHA, "labels_sha256": LABELS_SHA256, "scored_inventories_sha256": SCORES_SHA,
            "protocol_sha256": file_hash(HERE / "protocol.md"), "cases": cases}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    args = parser.parse_args()
    print(json.dumps(manifest(records(args.dataset)), indent=2))
