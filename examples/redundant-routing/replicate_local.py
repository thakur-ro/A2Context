"""Check two dispatch records across two new parcel codes and both source orders."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from propose_qwen import load_runtime, propose

MODEL = "Qwen/Qwen2.5-0.5B-Instruct"
RECORDS = ("Dispatch guide", "Routing note")
REPLACEMENTS = ("No dispatch record available.", "This parcel was scanned this morning.")


def answer(runtime, parcel, bay, order, edited=(), replacement=""):
    messages = [{"role": "system", "content": "Use the supplied dispatch records. Reply with the bay code alone. If no record specifies a bay, reply UNKNOWN."}]
    for record in order:
        value = replacement if record in edited else f"Route {parcel} parcels to bay {bay}."
        messages.append({"role": "user", "content": f"{record}: {value}"})
    messages.append({"role": "user", "content": f"Where should a {parcel} parcel go?"})
    request = {"model": MODEL, "messages": messages, "temperature": 0, "max_tokens": 16}
    try:
        return {"answer": propose(runtime, request, MODEL)["outcome"]}
    except ValueError as error:
        return {"unscored": str(error)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    runtime = load_runtime(args.model)
    cases = []
    with runtime.torch.inference_mode():
        for parcel, bay in [("Cedar", "IBIS"), ("Birch", "WREN")]:
            for order in [RECORDS, RECORDS[::-1]]:
                baseline = answer(runtime, parcel, bay, order)
                conditions = []
                for replacement in REPLACEMENTS:
                    arms = [answer(runtime, parcel, bay, order, edits, replacement)
                            for edits in [(RECORDS[0],), (RECORDS[1],), RECORDS]]
                    conditions.append({"replacement": replacement, "guide": arms[0], "note": arms[1], "both": arms[2]})
                cases.append({"parcel": parcel, "expected": bay, "order": order, "baseline": baseline, "conditions": conditions})
    print(json.dumps({"model": MODEL, "revision": args.revision, "runtime": runtime.identity,
                      "tokenizer_sha256": sha256((args.model / "tokenizer.json").read_bytes()).hexdigest(),
                      "temperature": 0, "max_tokens": 16, "cases": cases}, indent=2))


if __name__ == "__main__":
    main()
