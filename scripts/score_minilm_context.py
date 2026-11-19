"""Score context pairs in both directions with a local MiniLM NLI checkpoint."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer, __version__ as transformers_version


MODEL_ID = "cross-encoder/nli-MiniLM2-L6-H768"
REVISION = "b95119ce93d3e065de6214e38cd4a97b0f2f2c6d"
LABELS = ("contradiction", "entailment", "neutral")
TOKEN_LIMIT = 512
PAIRS_PER_BATCH = 8
SNAPSHOT = {
    "config.json": "8b0e41caff7567c0f53e6983f35591c3dec59507c9173ab125c5823394fb57f3",
    "model.safetensors": "ec9c5b6a08b60d7a01b465c7ed44b496244dfd1bed3274c95875a74cea89ab35",
    "tokenizer.json": "82139106e603ee4e1d5bc99d056ccbed5a92bc24848b1b5a7137c26e00d0dbf6",
    "tokenizer_config.json": "e06f5c3c435fc0255b08b5574cd574eec685390f60c85b27f83d785eb8ecaaa1",
    "special_tokens_map.json": "378eb3bf733eb16e65792d7e3fda5b8a4631387ca04d2015199c4d4f22ae554d",
}


def verify_snapshot(directory):
    for filename, expected in SNAPSHOT.items():
        digest = sha256()
        with (directory / filename).open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != expected:
            raise ValueError(f"{filename} differs from the pinned MiniLM snapshot.")


def score_pairs(tokenizer, model, pairs):
    """Skip overlong pairs and batch the complete inputs in both directions."""
    directions = [
        direction
        for pair in pairs
        for direction in ((pair["left"], pair["right"]), (pair["right"], pair["left"]))
    ]
    left, right = zip(*directions)
    encoded = tokenizer(list(left), list(right), truncation=False, verbose=False)["input_ids"]
    rows = [
        {"id": pair["id"], "status": "unscored", "reason": "too_long"}
        for pair in pairs
    ]
    eligible = [
        index
        for index in range(len(pairs))
        if max(len(encoded[index * 2]), len(encoded[index * 2 + 1])) <= TOKEN_LIMIT
    ]
    if not eligible:
        return rows
    input_ids = [encoded[index * 2 + direction] for index in eligible for direction in (0, 1)]
    batch = tokenizer.pad({"input_ids": input_ids}, padding=True, return_tensors="pt")
    scores = model(**batch).logits.softmax(dim=1).tolist()
    for position, index in enumerate(eligible):
        rows[index] = {
            "id": pairs[index]["id"],
            "status": "scored",
            "left_to_right": dict(zip(LABELS, scores[position * 2])),
            "right_to_left": dict(zip(LABELS, scores[position * 2 + 1])),
        }
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="Pinned local MiniLM snapshot")
    args = parser.parse_args()
    request = json.load(sys.stdin)
    if request["version"] != 1:
        raise ValueError("Expected a version 1 context scoring request.")
    verify_snapshot(args.model)
    torch.set_num_threads(4)
    tokenizer = AutoTokenizer.from_pretrained(args.model, local_files_only=True, trust_remote_code=False)
    model = AutoModelForSequenceClassification.from_pretrained(
        args.model, local_files_only=True, trust_remote_code=False, use_safetensors=True
    ).eval()
    if tuple(model.config.id2label[i] for i in range(3)) != LABELS or model.config.model_type != "roberta":
        raise ValueError("Use the pinned MiniLM checkpoint with its three NLI labels.")
    pairs = request["pairs"]
    rows = []
    with torch.inference_mode():
        for offset in range(0, len(pairs), PAIRS_PER_BATCH):
            rows.extend(score_pairs(tokenizer, model, pairs[offset : offset + PAIRS_PER_BATCH]))
    print(json.dumps({
        "version": 1,
        "scorer": {
            "model_id": MODEL_ID,
            "revision": REVISION,
            "runtime": f"transformers/{transformers_version}-torch/{torch.__version__}-cpu",
        },
        "pairs": rows,
    }, allow_nan=False))


if __name__ == "__main__":
    main()
