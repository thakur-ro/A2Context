"""Prompt alignment and decoder-block hooks for local MLX probes."""

from contextlib import contextmanager
from functools import partial

from activation_tokens import edited_prompt as align_edit, source_tokens


class BlockProbe:
    def __init__(self, block, observe):
        self.block = block
        self.observe = observe

    def __getattr__(self, name):
        return getattr(self.block, name)

    def __call__(self, *args, **kwargs):
        return self.observe(self.block(*args, **kwargs))


@contextmanager
def observe_blocks(layers, callbacks):
    original = {index: layers[index] for index in callbacks}
    try:
        for index, callback in callbacks.items():
            layers[index] = BlockProbe(original[index], callback)
        yield
    finally:
        for index, block in original.items():
            layers[index] = block


def prompt_input(tokenizer, messages, tools=None):
    prompt = tokenizer.apply_chat_template(messages, tools=tools, tokenize=False, add_generation_prompt=True)
    # Follow mlx-lm generation's BOS policy so the probe sees its exact token prefix.
    bos = tokenizer.bos_token
    add_special_tokens = bos is None or not prompt.startswith(bos)
    encoded = tokenizer._tokenizer(prompt, add_special_tokens=add_special_tokens, return_offsets_mapping=True)
    return prompt, encoded.input_ids, encoded.offset_mapping, add_special_tokens


def edited_prompt(tokenizer, messages, prefix, index, before, after, source_range, tools=None):
    return align_edit(
        partial(prompt_input, tokenizer), messages, prefix, index, before, after, source_range, tools,
    )


def patch_output(mx, donor, start, end):
    def observe(hidden):
        return mx.concatenate((hidden[:, :start], donor[:, start:end], hidden[:, end:]), axis=1)
    return observe
