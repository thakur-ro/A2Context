"""Score full-vocabulary first-token shifts for a NexusWeave prefill batch."""

import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import sys

from probe_transformers_answer_timeline import load_runtime


def first_step(runtime, ids):
    logits = runtime.model(input_ids=runtime.torch.tensor([ids]), use_cache=False, logits_to_keep=1).logits[0, -1]
    return logits.double().log_softmax(dim=0)


def js_bits(torch, p, q):
    log_mean = torch.logaddexp(p, q) - math.log(2)
    return ((p.exp() * (p - log_mean)).sum() + (q.exp() * (q - log_mean)).sum()).item() / (2 * math.log(2))


def score_batch(runtime, batch, model_id, revision, tokenizer_sha256):
    if batch["version"] != 1:
        raise ValueError("Expected a version 1 prefill batch.")

    def distribution(request):
        if request.keys() - {"messages", "tools", "model", "temperature", "max_tokens"}:
            raise ValueError("This adapter supports Qwen text messages and tool definitions.")
        if request.get("model", model_id) != model_id:
            raise ValueError("The requested model differs from the adapter.")
        _, ids, _, _ = runtime.prompt_input(request["messages"], request.get("tools"))
        return first_step(runtime, ids)

    baseline = distribution(batch["baseline"])
    effects = []
    for edit in batch["edits"]:
        edited = distribution(edit["request"])
        divergence = js_bits(runtime.torch, baseline, edited)
        if not -1e-12 <= divergence <= 1 + 1e-12:
            raise ValueError("Nonfinite or out-of-range first-token divergence.")
        effects.append({"context_id": edit["context_id"], "source_id": edit["source_id"], "source_version": edit["source_version"],
                        "request_sha256": edit["request_sha256"],
                        "js_bits": 0.0 if runtime.torch.equal(baseline, edited) else min(1.0, max(0.0, divergence))})
    return {"version": 1, "model_id": model_id, "model_revision": revision,
            "runtime": runtime.identity, "tokenizer_sha256": tokenizer_sha256,
            "baseline_request_sha256": batch["baseline_request_sha256"], "effects": effects}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    batch = json.load(sys.stdin)
    runtime = load_runtime(args.model)
    tokenizer_digest = sha256((args.model / "tokenizer.json").read_bytes()).hexdigest()
    with runtime.torch.inference_mode():
        scores = score_batch(runtime, batch, args.model_id, args.revision, tokenizer_digest)
    print(json.dumps(scores, allow_nan=False))
