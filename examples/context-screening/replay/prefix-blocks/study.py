"""Run single-position Qwen residual transplants on the frozen booking questions."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from check_question_holdout import verify_model
from probe_qwen_prefix_blocks import measure as measure_blocks
from probe_transformers_answer_timeline import load_runtime

PROTOCOL = HERE / "protocol.json"
CASES = HERE.parent / "prefix-indirect/cases.json"


def measure(runtime, record, pinned):
    envelope = record["envelope"]
    candidate = envelope["candidate"]
    scores = candidate["scores"]
    identity = ("model_id", "model_revision", "runtime", "tokenizer_sha256")
    if any(scores[k] != pinned[p] for k, p in zip(identity, ("checkpoint", "revision", "runtime", "tokenizer_sha256"))) or runtime.identity != scores["runtime"]:
        raise ValueError("The scorer identity differs from the pinned experiment.")
    ids = [tokens[0] for tokens in scores["candidate_token_ids"]]
    if [runtime.tokenizer.decode([i]) for i in ids] != ["3", "6"] or runtime.layer_count != 24:
        raise ValueError("The readout or decoder blocks differ from the protocol.")
    return measure_blocks(runtime, envelope["request"]["attributes"]["nexusweave.request.body"], candidate)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    protocol = json.loads(PROTOCOL.read_text())
    pinned = json.loads((HERE / protocol["model_protocol"]).read_text())
    if sha256(CASES.read_bytes()).hexdigest() != protocol["source_cases_sha256"]:
        raise ValueError("The original case file differs from the frozen protocol.")
    verify_model(args.model)
    if sha256((args.model / "tokenizer.json").read_bytes()).hexdigest() != pinned["tokenizer_sha256"]:
        raise ValueError("The tokenizer differs from the pinned tokenizer.")
    runtime = load_runtime(args.model)
    lookup = {r["id"]: r for r in json.loads(CASES.read_text())}
    reports = {}
    with runtime.torch.inference_mode():
        for key in protocol["cases"]:
            reports[key] = measure(runtime, lookup[key], pinned)
            print("Measured", key, flush=True)
    with args.output.open("x") as out:
        json.dump({"protocol_sha256": sha256(PROTOCOL.read_bytes()).hexdigest(), "reports": reports},
                  out, indent=2, allow_nan=False)
        out.write("\n")


if __name__ == "__main__":
    main()
