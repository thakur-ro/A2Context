"""Continue decoding freely after a source-token activation swap."""


def decode_generation_branches(case, runtime, inputs, baseline, sources, clean_states, donors):
    if not case.get("generation_branches"):
        raise ValueError("Declare at least one generation branch to measure.")
    spans = {source["context_id"]: source["edited_token_range"] for source in sources}
    branches = []
    seen = set()
    for spec in case["generation_branches"]:
        context_id, layer, start = (spec[key] for key in ("source_context_id", "layer", "start_output_token"))
        key = (context_id, layer, start)
        if key in seen or context_id not in spans or not 0 <= layer < runtime.layer_count or not 0 <= start < len(baseline):
            raise ValueError("Each branch needs a distinct aligned source, block, and output position.")
        seen.add(key)
        prefix = baseline[:start]
        max_new_tokens = case["request"]["max_tokens"] - start

        def finish(prompt_tokens, state=None):
            patch_steps = 0
            callbacks = {}
            if state is not None:
                patch = runtime.patch_output(state, *spans[context_id])

                def count_patch(hidden):
                    nonlocal patch_steps
                    patch_steps += 1
                    return patch(hidden)

                callbacks[layer] = count_patch
            new_tokens, stop = runtime.continue_greedily(prompt_tokens + prefix, max_new_tokens, callbacks)
            if state is not None and patch_steps != len(new_tokens) + int(stop == "end_of_sequence"):
                raise ValueError("The runtime did not apply the source swap at every decoding step.")
            output = prefix + new_tokens
            return {
                "text": runtime.tokenizer.decode(output, skip_special_tokens=False),
                "token_ids": output,
                "stop_reason": stop,
            }, patch_steps

        clean, clean_steps = finish(inputs[None], clean_states[layer])
        if clean["token_ids"] != baseline or clean["stop_reason"] != "end_of_sequence":
            raise ValueError("The clean-to-clean generation did not reproduce the captured output.")
        patched, patched_steps = finish(inputs[None], donors[context_id][1][layer])
        edited, _ = finish(inputs[context_id])
        branches.append({
            "source_context_id": context_id, "layer": layer, "start_output_token": start,
            "max_new_tokens": max_new_tokens, "clean": clean, "clean_patch_steps": clean_steps,
            "patched": patched, "patched_steps": patched_steps, "edited_prompt": edited,
        })
    return branches
