"""Score two fixed continuations against a batch of pending Qwen2 requests."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

from candidate_scoring import score_conditions
from probe_transformers_answer_timeline import load_runtime


def token_readouts(runtime, context, continuation):
    torch = runtime.torch
    positions = torch.arange(len(context) - 1, len(context) + len(continuation) - 1)
    logits = runtime.model(input_ids=torch.tensor([context + continuation[:-1]]),
                           use_cache=False, logits_to_keep=positions).logits[0]
    steps = []
    for row, token in zip(logits, continuation, strict=True):
        normalizer = row.logsumexp(dim=0)
        top = row.argmax().item()
        steps.append({"text": runtime.tokenizer.decode([token]), "log_prob": (row[token] - normalizer).item(),
                      "top_id": top, "top_text": runtime.tokenizer.decode([top]),
                      "top_log_prob": (row[top] - normalizer).item()})
    return steps


def score_batch(runtime, batch, model_id, revision, tokenizer_sha256, *, include_steps=False):
    return score_conditions(runtime, batch, model_id, revision, tokenizer_sha256,
                            read_steps=token_readouts if include_steps else None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--token-steps", action="store_true", help="Record the candidate and top token at each fixed prefix.")
    args = parser.parse_args()
    batch = json.load(sys.stdin)
    runtime = load_runtime(args.model)
    digest = sha256((args.model / "tokenizer.json").read_bytes()).hexdigest()
    with runtime.torch.inference_mode():
        report = score_batch(runtime, batch, args.model_id, args.revision, digest, include_steps=args.token_steps)
    print(json.dumps(report, allow_nan=False))


if __name__ == "__main__":
    main()
