# Where did the source swap change the next digit?

We took both pairs of activity questions where the same calendar edit restored one reference and lost another. All four unedited requests favored the next digit `3` over `6` after the supplied prefix `Activity length: `. The same source edit moved that preference toward `6` in all four. This follow-up asks whether transplanting one internal vector can reproduce that reversal. We fixed the questions, sites, readout, and controls before the block measurements. [Protocol](protocol.json) · [Underlying booking cases](../prefix-indirect/README.md)

At each of Qwen2.5-0.5B-Instruct's 24 decoder blocks, we took the post-block vector for one token from the edited prompt and substituted it at the corresponding token in the original prompt. We tried the last token of the user's message and the last token of the supplied answer prefix separately. The source replacements can change the prompt length. The runner records the aligned token IDs and positions on both sides. The readout is logit(3) minus logit(6) at the next position. Negative values favor 6. It covers the first digit of these two-token numbers.

| Bookings | Edited calendar | First block that reversed the digit preference at the prefix end, zero-based |
| --- | --- | --- |
| 3 and 4 | Atlas | Block 14 for the greenhouse question, block 22 for the rehearsal |
| 13 and 14 | Birch | Block 15 for the greenhouse question, block 22 for the rehearsal |

Swapping the final user-token vector never reversed the margin in any of the eight source-edit comparisons. For the other calendar, a prefix-end swap at block 14 reversed the greenhouse readout twice even though the full calendar edit preserved the preference. A block-level reversal alone would have given a misleading recommendation in those cases.

All self-swaps and terminal controls passed the fixed tolerance of 0.0001. Two CPU runs produced byte-identical reports. The terminal prefix-end swap checks that the donor state feeds the output head as expected. The earlier swaps measure sensitivity to these artificial hybrids. They cannot locate the first place the model learned the duration or establish which internal path it used in an unmodified run. These four questions were selected from one known template. [All margins, controls and position indices](results.json) · [Heimersheim and Nanda, activation patching guide](https://arxiv.org/abs/2404.15255v1)

To repeat the study, use the local Qwen weights and Python dependencies pinned in the [booking protocol](../prefix-indirect/protocol.json). From the checkout run:

```sh
study=examples/context-screening/replay/prefix-blocks
python3 "$study/study.py" --model "$CHECKPOINT_DIR" --output /tmp/calendar-blocks.json
cmp "$study/results.json" /tmp/calendar-blocks.json
```
