"""Check downloadable request bytes against both orders of the public QA passages."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import urlopen

from check_passage_edits import dataset_records, ordered_documents, request


def compact(body):
    return json.dumps(body, ensure_ascii=False, separators=(",", ":"))


def captured_inputs(dataset):
    folder = Path(__file__).parent
    studies = [json.loads((folder / name).read_text()) for name in ("results.json", "rotated-results.json")]
    inputs = {}
    for row, record in dataset_records(dataset):
        for study in studies:
            docs = ordered_documents(row, record, study["rotation"])
            passages = [doc["title"] + "\n" + doc["text"] for doc in docs]
            body = {"model": study["model"], "messages": request(record["question"], passages, None),
                    "temperature": 0, "max_tokens": 48}
            key = sha256(compact(body).encode()).hexdigest()
            inputs[key] = body, passages, record["question"]
    return inputs


def check_plan(plan, inputs):
    positions = [b["original_position"] for b in plan["backgrounds"]]
    if len(set(positions)) != 2:
        raise ValueError("Expected two distinct tested positions.")
    used = set()
    for bg in plan["backgrounds"]:
        body, original, question = inputs[bg["captured_request_sha256"]]
        others = original.copy()
        source = others.pop(bg["original_position"] - 1)
        if sha256(source.encode()).hexdigest() != plan["source"]["version"]:
            raise ValueError("Selected source differs from the captured passage.")
        expected_conditions = set()
        for slot in positions:
            passages = others.copy()
            passages.insert(slot - 1, source)
            for replaced in (False, True):
                raw = compact({**body, "messages": request(question, passages, slot-1 if replaced else None)})
                key = sha256(raw.encode()).hexdigest()
                if plan["requests"].get(key) != raw:
                    raise ValueError("Planned request differs from the independently assembled input.")
                expected_conditions.add((slot, replaced, key))
                used.add(key)
        actual = [(c["position"], c["source_replaced"], c["request_sha256"]) for c in bg["conditions"]]
        if len(actual) != 4 or set(actual) != expected_conditions:
            raise ValueError("The background has missing, duplicated, or mislabelled trial conditions.")
    if used != plan["requests"].keys():
        raise ValueError("Unused bodies in the downloaded plan.")


def check(base, dataset):
    def get(route, fields):
        with urlopen(base.rstrip("/") + "/api/sidecar/" + route + "?" + urlencode(fields), timeout=20) as resp:
            return json.load(resp)

    inputs = captured_inputs(dataset)
    offset, plans = 0, 0
    while offset is not None:
        page = get("replay-variations", {"org": "public-qa", "offset": offset})
        for row in page["rows"]:
            m = row["changed_example"]["measurement"]
            sel = {"org": "public-qa", "factor": row["factor"], "group": row["group"], "pair": row["position_pair"],
                   "baseline_trials": m["baseline"]["attempted"], "edit_trials": m["variant"]["attempted"]}
            plan = get("position-plan", sel)
            check_plan(plan, inputs)
            plans += 1
        offset = page["next_offset"]
    if not plans:
        raise ValueError("Load both saved studies before checking the position plans.")
    return plans


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    args = parser.parse_args()
    checked = check(args.url, args.dataset)
    print(f"Verified {checked} plans and {8 * checked} conditions against independently reconstructed request bytes.")
