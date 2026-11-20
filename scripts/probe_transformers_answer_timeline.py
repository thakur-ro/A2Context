"""Measure a Qwen2 activation timeline with Transformers and PyTorch on CPU."""

import argparse
from contextlib import ExitStack, contextmanager
from importlib.metadata import version
import json
from pathlib import Path

from activation_timeline import measure, tool_payload


class TransformersRuntime:
    def __init__(self, model, tokenizer, torch, generation_config):
        self.model = model
        self.tokenizer = tokenizer
        self.torch = torch
        self.generation_config = generation_config
        self.layer_count = len(model.model.layers)
        self.identity = f"Transformers {version('transformers')} / PyTorch {version('torch')} / CPU float32"

    def render_prompt(self, messages, tools):
        return self.tokenizer.apply_chat_template(messages, tools=tools, tokenize=False, add_generation_prompt=True)

    def prompt_input(self, messages, tools):
        prompt = self.render_prompt(messages, tools)
        encoded = self.tokenizer(prompt, add_special_tokens=False, return_offsets_mapping=True)
        return prompt, encoded.input_ids, encoded.offset_mapping, False

    def generate(self, _prompt, tokens, request, *, seed=None):
        sampling = {}
        if seed is not None:
            if request["temperature"] <= 0:
                raise ValueError("Seeded sampling requires a positive temperature.")
            self.torch.manual_seed(seed)
            sampling = dict(do_sample=True, temperature=request["temperature"], top_k=0, top_p=1.0)
        elif request["temperature"] != 0:
            raise ValueError("Supply a sampling seed for a nonzero temperature.")
        inputs = self.torch.tensor([tokens])
        generated = self.model.generate(
            input_ids=inputs,
            attention_mask=self.torch.ones_like(inputs),
            generation_config=self.generation_config,
            max_new_tokens=request["max_tokens"],
            **sampling,
        )[0, len(tokens):].tolist()
        if not generated or generated[-1] not in self.generation_config.eos_token_id:
            raise ValueError("The fixture answer must finish before the generation limit.")
        generated.pop()
        return self.tokenizer.decode(generated, skip_special_tokens=False), generated

    @staticmethod
    def parse_proposal(answer, tools, *, normalize_template_braces=True):
        payload, position = tool_payload(answer, "<tool_call>", "</tool_call>")
        if payload is None:
            return None, None
        payload = payload.strip()
        # Qwen2.5's tool template example wraps its JSON object in an extra brace pair.
        if normalize_template_braces and payload.startswith("{{") and payload.endswith("}}"):
            payload = payload[1:-1]
        try:
            proposal = json.loads(payload)
        except json.JSONDecodeError as error:
            raise ValueError("The proposed Qwen2 tool call has invalid JSON.") from error
        names = [tool["function"]["name"] for tool in tools]
        if (not isinstance(proposal, dict) or proposal.keys() != {"name", "arguments"}
                or proposal["name"] not in names or not isinstance(proposal["arguments"], dict)):
            raise ValueError("Expected one proposed call to a listed tool with named arguments.")
        return proposal, position

    @contextmanager
    def observe_blocks(self, callbacks):
        with ExitStack() as hooks:
            for block, callback in callbacks.items():
                layer = self.model.model.layers[block]
                handle = layer.register_forward_hook(
                    lambda _layer, _input, output, observe=callback: observe(output),
                )
                hooks.callback(handle.remove)
            yield

    def scores(self, tokens, readouts, callbacks, **model_inputs):
        with self.observe_blocks(callbacks):
            positions = self.torch.tensor([position for position, _, _ in readouts])
            logits = self.model(
                input_ids=self.torch.tensor([tokens]), use_cache=False, logits_to_keep=positions, **model_inputs,
            ).logits
            scores = []
            for row, (_, first, second) in zip(logits[0], readouts, strict=True):
                reference = row.logsumexp(dim=0) if second is None else row[second]
                scores.append((row[first] - reference).item())
            return scores

    def continue_greedily(self, tokens, max_new_tokens, callbacks):
        inputs = self.torch.tensor([tokens])
        with self.observe_blocks(callbacks):
            generated = self.model.generate(
                input_ids=inputs, attention_mask=self.torch.ones_like(inputs),
                generation_config=self.generation_config, max_new_tokens=max_new_tokens,
                use_cache=False,
            )[0, len(tokens):].tolist()
        eos = bool(generated) and generated[-1] in self.generation_config.eos_token_id
        if eos:
            generated.pop()
        return generated, "end_of_sequence" if eos else "token_limit"

    @staticmethod
    def patch_output(donor, start, end):
        def observe(hidden):
            patched = hidden.clone()
            patched[:, start:end] = donor[:, start:end]
            return patched
        return observe


def load_runtime(model_path):
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, GenerationConfig

    torch.set_num_threads(4)
    tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True, trust_remote_code=False)
    model = AutoModelForCausalLM.from_pretrained(
        model_path, local_files_only=True, trust_remote_code=False, use_safetensors=True,
        dtype=torch.float32, attn_implementation="eager",
    ).eval()
    if model.config.model_type != "qwen2":
        raise ValueError("The Transformers adapter instruments Qwen2 decoder blocks.")
    eos = model.generation_config.eos_token_id
    eos = [eos] if isinstance(eos, int) else eos
    generation = GenerationConfig(eos_token_id=eos, pad_token_id=tokenizer.pad_token_id, do_sample=False)
    return TransformersRuntime(model, tokenizer, torch, generation)


def probe(case, model_path, model_id, revision, *, all_output_tokens=False, decode_branches=False):
    runtime = load_runtime(model_path)
    with runtime.torch.inference_mode():
        return measure(case, runtime, model_path, model_id, revision,
                       all_output_tokens=all_output_tokens, decode_branches=decode_branches)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--case", type=Path, default=Path("examples/runtime-comparison/case-transformers.json"))
    parser.add_argument("--all-output-tokens", action="store_true")
    parser.add_argument("--decode-branches", action="store_true")
    args = parser.parse_args()
    case = json.loads(args.case.read_text())
    report = probe(case, args.model, args.model_id, args.revision,
                   all_output_tokens=args.all_output_tokens, decode_branches=args.decode_branches)
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
