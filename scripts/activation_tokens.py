"""Token alignment helpers for activation probes."""

from copy import deepcopy


def source_tokens(prompt, offsets, message):
    if prompt.count(message) != 1:
        raise ValueError("A measured source must occur once in the rendered prompt.")
    start = prompt.index(message)
    end = start + len(message)
    positions = [
        index for index, (left, right) in enumerate(offsets)
        if left < end and start < right and left < right
    ]
    if not positions:
        raise ValueError("The captured source has no input token alignment.")
    return [positions[0], positions[-1] + 1]


def edited_prompt(render, messages, prefix, index, before, after, source_range, tools=None):
    changed = deepcopy(messages)
    content = changed[index]["content"]
    if content.count(before) != 1:
        raise ValueError(f"Expected exactly one {before!r} in message {index}.")
    changed[index]["content"] = content.replace(before, after)
    prompt, tokens, _, _ = render(changed, tools)
    if len(tokens) != len(prefix):
        raise ValueError(f"The {before!r} edit changes the length of the token prefix.")
    positions = [index for index, (a, b) in enumerate(zip(prefix, tokens)) if a != b]
    if not positions or not all(source_range[0] <= index < source_range[1] for index in positions):
        raise ValueError(f"The {before!r} edit is not isolated to the captured source.")
    return prompt, tokens, [positions[0], positions[-1] + 1]
