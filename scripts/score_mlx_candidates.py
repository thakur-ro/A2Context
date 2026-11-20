"""Score pending LFM2 requests on Apple Silicon using the captured message and tool format."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

from candidate_scoring import score_conditions
from probe_mlx_answer_timeline import MlxRuntime


class CandidateRuntime(MlxRuntime):
    def render_prompt(self, messages, tools):
        prompt = self.tokenizer.apply_chat_template(messages, tools=tools, tokenize=False, add_generation_prompt=True)
        if self.tokenizer.bos_token and not prompt.startswith(self.tokenizer.bos_token):
            raise ValueError("The LFM template must supply its initial token.")
        return prompt


def token_readouts(runtime, context, continuation):
    mx = runtime.mx
    logits = runtime.model(mx.array([context + continuation[:-1]]))[0,len(context)-1:].astype(mx.float32)
    log_probs = logits - mx.logsumexp(logits, axis=-1, keepdims=True)
    tops = mx.argmax(log_probs, axis=1).tolist()
    steps = []
    for i, (token, top) in enumerate(zip(continuation, tops, strict=True)):
        steps.append({"text":runtime.tokenizer.decode([token]), "log_prob":float(log_probs[i,token].item()),
                      "top_id":top, "top_text":runtime.tokenizer.decode([top]), "top_log_prob":float(log_probs[i,top].item())})
    return steps


def main():
    import mlx.core as mx
    from mlx_lm import load

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--token-steps", action="store_true")
    args = parser.parse_args()
    if not args.model.is_dir():
        parser.error("Supply a local model directory.")
    model, tokenizer = load(str(args.model.resolve()))
    if model.model_type != "lfm2":
        parser.error("This adapter supports LFM2 checkpoints.")
    runtime = CandidateRuntime(model, tokenizer, mx)
    report = score_conditions(runtime, json.load(sys.stdin), args.model_id, args.revision,
                              sha256((args.model / "tokenizer.json").read_bytes()).hexdigest(),
                              read_steps=token_readouts if args.token_steps else None)
    print(json.dumps(report, allow_nan=False))


if __name__ == "__main__":
    main()
