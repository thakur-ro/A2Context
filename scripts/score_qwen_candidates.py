"""Score two fixed continuations against a batch of pending Qwen2 requests."""

import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import sys

from probe_transformers_answer_timeline import load_runtime


def score_batch(runtime, batch, model_id, revision, tokenizer_sha256):
    if batch["version"] != 1:
        raise ValueError("Candidate scoring requires version 1.")
    def encode(text):
        return runtime.tokenizer.encode(text, add_special_tokens=False)

    prefix = batch["assistant_prefix"]
    continuations = [candidate["continuation"] for candidate in batch["candidates"]]
    expected = None
    rows = []
    for condition in batch["conditions"]:
        request = condition["request"]
        if request.keys() - {"messages", "tools", "model", "temperature", "max_tokens"}:
            raise ValueError("This adapter supports Qwen2 text messages and tool definitions.")
        if request.get("model", model_id) != model_id:
            raise ValueError("The request names a different model from the scorer.")
        rendered = runtime.render_prompt(request["messages"], request.get("tools"))
        prompt_tokens = encode(rendered)
        context = encode(rendered + prefix)
        if context[:len(prompt_tokens)] != prompt_tokens:
            raise ValueError("The assistant prefix crosses a prompt token boundary.")
        prefix_tokens = context[len(prompt_tokens):]
        candidates = []
        for text in continuations:
            sequence = encode(rendered + prefix + text)
            if sequence[:len(context)] != context or len(sequence) == len(context):
                raise ValueError("Each candidate must start on a token boundary after the fixed prefix.")
            candidates.append(sequence[len(context):])
        if expected is None:
            expected = prefix_tokens, candidates
        elif expected != (prefix_tokens, candidates):
            raise ValueError("A source edit changed the fixed prefix or candidate tokens.")

        first, second = candidates
        if first[:len(second)] == second or second[:len(first)] == first:
            raise ValueError("Candidate token sequences must be disjoint.")
        log_probs = []
        for tokens in candidates:
            readouts = [(len(context) + index - 1, token, None) for index, token in enumerate(tokens)]
            token_scores = runtime.scores(context + tokens[:-1], readouts, {})
            log_probs.append(math.fsum(token_scores))
        rows.append({
            "id": condition["id"],
            "rendered_prefix_sha256": sha256((rendered + prefix).encode()).hexdigest(),
            "log_probs": log_probs,
        })
    if expected is None:
        raise ValueError("Candidate scoring requires at least one request condition.")
    return {
        "version": 1, "model_id": model_id, "model_revision": revision, "runtime": runtime.identity,
        "tokenizer_sha256": tokenizer_sha256, "assistant_prefix_token_ids": expected[0],
        "candidate_token_ids": expected[1], "conditions": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    batch = json.load(sys.stdin)
    runtime = load_runtime(args.model)
    digest = sha256((args.model / "tokenizer.json").read_bytes()).hexdigest()
    with runtime.torch.inference_mode():
        report = score_batch(runtime, batch, args.model_id, args.revision, digest)
    print(json.dumps(report, allow_nan=False))


if __name__ == "__main__":
    main()
