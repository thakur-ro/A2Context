"""Measure prompt-source interventions at successive tokens on a fixed generation path."""

import argparse
from importlib.metadata import version
import json
import os
from pathlib import Path

from activation_timeline import measure
from mlx_activation import observe_blocks, patch_output, prompt_input


def output_tokens(model, tokenizer, prompt, request):
    from mlx_lm import stream_generate
    from mlx_lm.sample_utils import make_sampler

    events = list(stream_generate(
        model, tokenizer, prompt, max_tokens=request["max_tokens"],
        sampler=make_sampler(temp=request["temperature"]),
    ))
    if events[-1].finish_reason != "stop":
        raise ValueError("The fixture answer must finish before the generation limit.")
    return "".join(event.text for event in events), [event.token for event in events[:-1]]


def parse_proposal(tokenizer, answer, tools):
    if not tokenizer.tool_call_start:
        raise ValueError("The model has no tool parser.")
    leading, start, body = answer.partition(tokenizer.tool_call_start)
    if not start:
        return None, None
    payload, end, tail = body.partition(tokenizer.tool_call_end)
    if not end or tokenizer.tool_call_start in tail:
        raise ValueError("Expected one complete proposed tool call.")
    parsed = tokenizer.tool_parser(payload, tools)
    names = {tool["function"]["name"] for tool in tools}
    if not isinstance(parsed, dict) or parsed["name"] not in names or not isinstance(parsed["arguments"], dict):
        raise ValueError("Expected one proposed call to a listed tool with named arguments.")
    payload_start = len(leading) + len(start)
    return parsed, slice(payload_start, payload_start + len(payload))


class MlxRuntime:
    def __init__(self, model, tokenizer, mx):
        self.model = model
        self.tokenizer = tokenizer
        self.mx = mx
        self.layer_count = len(model.layers)
        self.identity = "mlx-lm " + version("mlx-lm")

    def prompt_input(self, messages, tools):
        return prompt_input(self.tokenizer, messages, tools)

    def generate(self, prompt, _tokens, request):
        return output_tokens(self.model, self.tokenizer, prompt, request)

    def parse_proposal(self, answer, tools):
        return parse_proposal(self.tokenizer, answer, tools)

    def scores(self, tokens, readouts, callbacks):
        with observe_blocks(self.model.layers, callbacks):
            logits = self.model(self.mx.array([tokens]))
            scores = []
            for position, first, second in readouts:
                row = logits[0, position].astype(self.mx.float32)
                reference = self.mx.logsumexp(row) if second is None else row[second]
                scores.append(float((row[first] - reference).item()))
            return scores

    def patch_output(self, donor, start, end):
        return patch_output(self.mx, donor, start, end)


def probe(case, model_path, model_id, revision, *, all_output_tokens=False):
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    import mlx.core as mx
    from mlx_lm import load

    model, tokenizer = load(str(model_path))
    if model.model_type != "lfm2":
        raise ValueError("This probe uses the MLX-LM LFM2 decoder blocks.")
    return measure(case, MlxRuntime(model, tokenizer, mx), model_path, model_id, revision,
                   all_output_tokens=all_output_tokens)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--case", type=Path, default=Path("examples/local-mlx-timeline/case.json"))
    parser.add_argument("--all-output-tokens", action="store_true")
    args = parser.parse_args()
    case = json.loads(args.case.read_text())
    report = probe(case, args.model, args.model_id, args.revision, all_output_tokens=args.all_output_tokens)
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
