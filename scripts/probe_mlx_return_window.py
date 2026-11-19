"""Measure decoder-block activation interventions on the local return-window fixture."""

import argparse
from hashlib import sha256
from importlib.metadata import version
import json
import os
from pathlib import Path
import re

from mlx_activation import edited_prompt, observe_blocks, patch_output, prompt_input, source_tokens


def differing_choice(tokenizer, prompt, prefix, add_special_tokens):
    continuations = [
        tokenizer.encode(prompt + answer, add_special_tokens=add_special_tokens)
        for answer in ("30", "60")
    ]
    if any(ids[:len(prefix)] != prefix for ids in continuations):
        raise ValueError("The candidate answers change the tokenization of the fixed prefix.")
    first = next(
        (index for index, (left, right) in enumerate(zip(*continuations)) if left != right),
        None,
    )
    if first is None:
        raise ValueError("The candidate answers lack distinct next tokens.")
    return continuations[0][len(prefix):first], [ids[first] for ids in continuations]


def probe(case, model_path, model_id, revision):
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    import mlx.core as mx
    from mlx_lm import generate, load
    from mlx_lm.sample_utils import make_sampler

    model, tokenizer = load(str(model_path))
    if model.model_type != "lfm2":
        raise ValueError("This probe uses the MLX-LM LFM2 decoder blocks.")
    messages = case["request"]["messages"]
    prompt, tokens, offsets, special = prompt_input(tokenizer, messages)
    policy = source_tokens(prompt, offsets, messages[0]["content"])
    store = source_tokens(prompt, offsets, messages[1]["content"])
    common, choice_tokens = differing_choice(tokenizer, prompt, tokens, special)
    policy_prompt, policy_tokens, policy_edit = edited_prompt(
        tokenizer, messages, tokens, 0, "30", "60", policy,
    )
    store_prompt, store_tokens, store_edit = edited_prompt(
        tokenizer, messages, tokens, 1, "Alder", "Cedar", store,
    )
    for changed_prompt, changed_tokens in [(policy_prompt, policy_tokens), (store_prompt, store_tokens)]:
        if differing_choice(tokenizer, changed_prompt, changed_tokens, special) != (common, choice_tokens):
            raise ValueError("A donor prompt changes the candidate answer tokenization.")

    arrays = {
        name: mx.array([prefix + common])
        for name, prefix in [("baseline", tokens), ("policy_edit", policy_tokens), ("store_edit", store_tokens)]
    }
    position = len(tokens) + len(common) - 1
    layers = model.layers

    def margin(name, callbacks=None):
        with observe_blocks(layers, callbacks or {}):
            logits = model(arrays[name])[0, position].astype(mx.float32)
            return float((logits[choice_tokens[0]] - logits[choice_tokens[1]]).item())

    def capture(name):
        states = {}

        def save(index):
            def observe(hidden):
                states[index] = hidden
                return hidden
            return observe

        score = margin(name, {index: save(index) for index in range(len(layers))})
        mx.eval(*states.values())
        return score, states

    baseline, clean_states = capture("baseline")
    changed, policy_states = capture("policy_edit")
    control, store_states = capture("store_edit")
    midpoint = len(layers) // 2
    clean_check = margin("baseline", {midpoint: patch_output(mx, clean_states[midpoint], *policy_edit)})
    donor_check = margin("baseline", {midpoint: patch_output(mx, policy_states[midpoint], 0, position + 1)})
    if abs(clean_check - baseline) > 0.05 or abs(donor_check - changed) > 0.05:
        raise RuntimeError("Activation probes failed the clean and full-donor checks.")

    sweeps = []
    for context_id, target, donor, span in [
        ("returns-policy", "source_tokens", policy_states, policy_edit),
        ("store-name", "source_tokens", store_states, store_edit),
        ("returns-policy", "prediction_token", policy_states, [position, position + 1]),
    ]:
        scores = [margin("baseline", {index: patch_output(mx, donor[index], *span)}) for index in range(len(layers))]
        sweeps.append({"source_context_id": context_id, "target": target, "token_range": span, "margins": scores})

    def greedy_answer(text):
        response = generate(model, tokenizer, prompt=text, max_tokens=20,
                            sampler=make_sampler(temp=0.0), verbose=False)
        match = re.fullmatch(r"\s*(\d+)(?:\s+days?)?\.?\s*", response, flags=re.IGNORECASE)
        if not match:
            raise RuntimeError(f"Expected a plain return-window answer, got {response!r}.")
        return int(match[1])

    return {
        "version": 1,
        "id": "lfm-return-window-activation",
        "location": case["location"],
        "case_id": case["id"],
        "model_id": model_id,
        "model_revision": revision,
        "runtime": "mlx-lm " + version("mlx-lm"),
        "prompt_sha256": sha256(prompt.encode()).hexdigest(),
        "tokenizer_sha256": sha256((model_path / "tokenizer.json").read_bytes()).hexdigest(),
        "readout": "next_token_log_odds",
        "alternatives": ["30", "60"],
        "choice_token_ids": choice_tokens,
        "prediction_position": position,
        "layer_count": len(layers),
        "baseline": {"answer": greedy_answer(prompt), "margin": baseline},
        "checks": {
            "layer": midpoint,
            "source_context_id": "returns-policy",
            "clean_patch_margin": clean_check,
            "full_donor_margin": donor_check,
        },
        "sources": [
            {
                "context_id": context_id,
                "captured_text_sha256": sha256(messages[index]["content"].encode()).hexdigest(),
                "token_range": span,
                "edited_token_range": edited_span,
                "before": before,
                "after": after,
                "edited_outcome": {"answer": greedy_answer(edited), "margin": score},
            }
            for index, context_id, span, edited_span, before, after, edited, score in [
                (0, "returns-policy", policy, policy_edit, "30", "60", policy_prompt, changed),
                (1, "store-name", store, store_edit, "Alder", "Cedar", store_prompt, control),
            ]
        ],
        "sweeps": sweeps,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="Local MLX model directory")
    parser.add_argument("--model-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--case", type=Path, default=Path("examples/local-mlx/case.json"))
    args = parser.parse_args()
    if not args.model.is_dir():
        parser.error("a local model directory is required")
    case = json.loads(args.case.read_text())
    print(json.dumps(probe(case, args.model, args.model_id, args.revision), indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
