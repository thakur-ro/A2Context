"""Measure a version 2 activation timeline using a local runtime adapter."""

from hashlib import sha256

from activation_tokens import edited_prompt, source_tokens
from activation_branches import decode_generation_branches


def tool_payload(answer, open_tag, close_tag):
    leading, start, body = answer.partition(open_tag)
    if not start:
        return None, None
    payload, end, tail = body.partition(close_tag)
    if not end or open_tag in tail:
        raise ValueError("Expected one complete proposed tool call.")
    start_at = len(leading) + len(start)
    return payload, slice(start_at, start_at + len(payload))


def checkpoint_specs(tokenizer, prompt, prompt_tokens, special, answer, generated, specs, measured):
    clean = prompt_tokens + generated
    if tokenizer.encode(prompt + answer, add_special_tokens=special) != clean:
        raise ValueError("The generated answer must round-trip through the tokenizer.")
    checkpoints = []
    for spec in specs:
        before, after = spec["before"], spec["after"]
        if answer[measured].count(before) != 1:
            raise ValueError(f"Expected exactly one {before!r} in the measured output.")
        counterfactual = answer[:measured.start] + answer[measured].replace(before, after, 1) + answer[measured.stop:]
        alternate = tokenizer.encode(prompt + counterfactual, add_special_tokens=special)
        index = next((i for i, pair in enumerate(zip(clean, alternate)) if pair[0] != pair[1]), None)
        if index is None or index < len(prompt_tokens):
            raise ValueError(f"The {before!r} contrast needs a prediction inside the answer.")
        output_index = index - len(prompt_tokens)
        prefix = tokenizer.decode(generated[:output_index], skip_special_tokens=False)
        # Require that each token pair represents the complete contrast, not a shared fragment.
        if any(
            not text.startswith(prefix + tokenizer.decode([ids[index]], skip_special_tokens=False))
            or tokenizer.decode([ids[index]], skip_special_tokens=False).strip() != value
            for text, ids, value in [(answer, clean, before), (counterfactual, alternate, after)]
        ):
            raise ValueError(f"The {before!r} contrast does not align with a whole output token.")
        checkpoints.append({
            "label": spec["label"], "output_token": output_index, "answer_prefix": prefix,
            "alternatives": [before, after], "choice_token_ids": [clean[index], alternate[index]],
        })
    return checkpoints


def output_fragments(tokenizer, answer, generated):
    cursor = 0
    fragments = []
    for index in range(1, len(generated) + 1):
        prefix = tokenizer.decode(generated[:index], skip_special_tokens=False)
        end = len(prefix) if answer.startswith(prefix) and len(prefix) >= cursor else cursor
        fragments.append(answer[cursor:end])
        cursor = end
    if cursor != len(answer):
        raise ValueError("The generated token fragments do not cover the original output.")
    return fragments


def measure(case, runtime, model_path, model_id, revision, *, all_output_tokens=False, decode_branches=False):
    tokenizer = runtime.tokenizer
    request = case["request"]
    messages = request["messages"]
    tools = request.get("tools")
    prompt, tokens, offsets, special = runtime.prompt_input(messages, tools)
    answer, generated = runtime.generate(prompt, tokens, request)
    tool_calls = None
    measured = slice(0, len(answer))
    if tools:
        proposal, measured = runtime.parse_proposal(answer, tools)
        if proposal is None:
            raise ValueError("Expected the original model output to propose a tool call.")
        tool_calls = {"baseline": proposal, "edited": {}}
    specs = case.get("checkpoints", [])
    if not case["edits"]:
        raise ValueError("Supply at least one source edit to measure.")
    if not all_output_tokens and len(specs) < 2:
        raise ValueError("Supply two checkpoint contrasts or enable full output token scoring.")
    checkpoints = checkpoint_specs(tokenizer, prompt, tokens, special, answer, generated, specs, measured)
    layer_count = runtime.layer_count
    sources = []
    aligned_inputs = {None: tokens}
    for edit in case["edits"]:
        context_id, index, before, after = (edit[key] for key in ("context_id", "message_index", "before", "after"))
        if context_id in aligned_inputs:
            raise ValueError("Each measured source must have a distinct context ID.")
        content = messages[index]["content"]
        span = source_tokens(prompt, offsets, content)
        edited, aligned, changed = edited_prompt(runtime.prompt_input, messages, tokens, index, before, after, span, tools)
        edited_answer, _ = runtime.generate(edited, aligned, request)
        if tool_calls is not None:
            proposal, _ = runtime.parse_proposal(edited_answer, tools)
            tool_calls["edited"][context_id] = proposal
        aligned_inputs[context_id] = aligned
        sources.append({
            "context_id": context_id, "before": before, "after": after,
            "captured_text_sha256": sha256(content.encode()).hexdigest(),
            "token_range": span, "edited_token_range": changed, "edited_outcome": edited_answer,
        })

    positions = [len(tokens) + step["output_token"] - 1 for step in checkpoints]
    # Actual baseline output tokens are supplied to all runs, including prompt edits.
    # The causal mask ensures tokens after a readout are invisible to that readout.
    continuation = generated[:-1] if all_output_tokens else generated[:max(step["output_token"] for step in checkpoints)]
    inputs = {key: prefix + continuation for key, prefix in aligned_inputs.items()}
    readouts = [(position, *step["choice_token_ids"]) for position, step in zip(positions, checkpoints)]
    token_score_start = len(readouts)
    if all_output_tokens:
        readouts.extend((len(tokens) + index - 1, token, None) for index, token in enumerate(generated))

    def scores(key, callbacks):
        return runtime.scores(inputs[key], readouts, callbacks)

    def capture(key):
        states = [None] * layer_count

        def save(index):
            def observe(hidden):
                states[index] = hidden
                return hidden
            return observe

        margins = scores(key, {index: save(index) for index in range(layer_count)})
        if any(state is None for state in states):
            raise RuntimeError("The runtime did not return an activation at every measured block.")
        return margins, states

    def swapped(block, donor, span):
        return scores(None, {block: runtime.patch_output(donor[block], *span)})

    baseline, clean_states = capture(None)
    donors = {source["context_id"]: capture(source["context_id"]) for source in sources}
    source_sweeps = [
        [swapped(block, donors[source["context_id"]][1], source["edited_token_range"]) for block in range(layer_count)]
        for source in sources
    ]
    midpoint = layer_count // 2
    for step_index, (step, spec, position) in enumerate(zip(checkpoints, specs, positions)):
        control_id = spec["control_donor"]
        donor_margins, donor_states = donors[control_id]
        prediction_span = [position, position + 1]
        clean = swapped(midpoint, clean_states, prediction_span)[step_index]
        full = swapped(midpoint, donor_states, [0, position + 1])[step_index]
        if abs(clean - baseline[step_index]) > 0.05 or abs(full - donor_margins[step_index]) > 0.05:
            raise ValueError("The clean or full-donor control failed at a checkpoint.")
        step.update(
            baseline_margin=baseline[step_index],
            edited_margins={key: values[0][step_index] for key, values in donors.items()},
            checks={"layer": midpoint, "source_context_id": control_id, "clean_patch_margin": clean, "full_donor_margin": full},
            sweeps=[
                {
                    "source_context_id": source["context_id"], "target": "source_tokens",
                    "token_range": source["edited_token_range"], "margins": [row[step_index] for row in sweep],
                }
                for source, sweep in zip(sources, source_sweeps)
            ] + [{
                "source_context_id": control_id, "target": "prediction_token", "token_range": prediction_span,
                "margins": [swapped(block, donor_states, prediction_span)[step_index] for block in range(layer_count)],
            }],
        )
    report = {
        "version": 2, "id": case["id"] + "-activation", "location": case["location"], "case_id": case["id"],
        "model_id": model_id, "model_revision": revision, "runtime": runtime.identity,
        "prompt_sha256": sha256(prompt.encode()).hexdigest(),
        "tokenizer_sha256": sha256((model_path / "tokenizer.json").read_bytes()).hexdigest(),
        "readout": "next_token_log_odds" if checkpoints else "output_token_log_probability",
        "layer_count": layer_count,
        "baseline_answer": answer, "prompt_token_count": len(tokens), "output_token_ids": generated,
        "sources": sources, "checkpoints": checkpoints,
    }
    if tool_calls is not None:
        report["tool_calls"] = tool_calls
    if all_output_tokens:
        control_id = specs[0]["control_donor"] if specs else sources[0]["context_id"]
        donor_scores, donor_states = donors[control_id]
        prefix_span = [0, len(tokens) + len(continuation)]
        clean = swapped(midpoint, clean_states, prefix_span)[token_score_start:]
        full = swapped(midpoint, donor_states, prefix_span)[token_score_start:]
        if any(
            abs(actual - expected) > 0.05
            for measured, reference in [(clean, baseline), (full, donor_scores)]
            for actual, expected in zip(measured, reference[token_score_start:], strict=True)
        ):
            raise ValueError("The clean or full-donor control failed on an output token.")
        report["output_scores"] = {
            "readout": "output_token_log_probability",
            "fragments": output_fragments(tokenizer, answer, generated),
            "baseline_log_probs": baseline[token_score_start:],
            "sources": {
                source["context_id"]: {
                    "edited_log_probs": donors[source["context_id"]][0][token_score_start:],
                    "block_log_probs": [row[token_score_start:] for row in sweep],
                }
                for source, sweep in zip(sources, source_sweeps, strict=True)
            },
            "checks": {
                "layer": midpoint, "source_context_id": control_id,
                "clean_patch_log_probs": clean, "full_donor_log_probs": full,
            },
        }
        report["id"] += "-output-tokens"
    if decode_branches:
        report["generation_branches"] = decode_generation_branches(
            case, runtime, aligned_inputs, generated, sources, clean_states, donors,
        )
        report["id"] += "-branches"
    return report
