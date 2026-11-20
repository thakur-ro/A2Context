"""Print a score-blind reading packet from the pinned upstream QA file."""
import gzip
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from annotation_gap import DATA_SHA, digest, file_hash

ROWS = (*range(2500, 2651, 25), *range(2512, 2613, 25))


def packets(dataset):
    if file_hash(dataset) != DATA_SHA:
        raise ValueError("Dataset digest differs from the omission experiment")
    with gzip.open(dataset, "rt") as records:
        for row, line in enumerate(records):
            if row not in ROWS:
                continue
            case = json.loads(line)
            docs = [doc["title"] + "\n" + doc["text"] for doc in case["ctxs"]]
            yield {"row": row, "question": case["question"], "aliases": case["answers"],
                   "passages": [{"id": digest(text), "text": text} for text in sorted(docs, key=digest)]}


if __name__ == "__main__":
    for packet in packets(Path(sys.argv[1])):
        print(json.dumps(packet, ensure_ascii=False))
