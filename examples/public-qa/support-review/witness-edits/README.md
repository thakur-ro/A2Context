# What changes when all reviewed supporting evidence is removed?

On 12 previously reviewed questions, removing passages that covered every recorded route to an answer changed Qwen's exact response on **10 of 12 questions**. Removing the same number of matched control passages changed six responses. The target edit alone changed five answers, the control alone changed one, both edits changed five, and neither changed one. All 36 answers reached an end token. [Frozen selection protocol](protocol.md) · [Exact answers, stop reasons and costs](answers.json)

| Condition | Completed | Changed from original | Replied UNKNOWN | Contained a reference alias | Prompt tokens | Generated tokens including stop | Local seconds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Original nine passages | 12 | 0 | 0 | 2 | 16,292 | 122 | 16.25 |
| Remove reviewed evidence | 12 | 10 | 0 | 2 | 13,477 | 142 | 14.12 |
| Remove matched controls | 12 | 6 | 0 | 3 | 13,285 | 152 | 14.16 |

Row 2575 lost a reference match. With all nine passages the answer was “Ricky Nelson.” Removing the reviewed evidence produced “Gary Hunley,” while the control edit preserved “Ricky Nelson.” On row 2550 the evidence edit only added quotation marks around a song title. Both count equally as text changes. On row 2512 the answer stayed “Chris Rea” in all three conditions. This was the only question where the two edited strings were identical. [Wikipedia contributors and source articles for each numbered row](../../data-credits.md#article-sources-by-dataset-row)

One AI reviewer previously identified sufficient and uncertain evidence in a pool of ten passages. We used the nine original passages actually present in each request. Every target edit interrupted all the reference and alternative answer derivations that reviewer recorded in that inventory. The control drew only from passages outside those annotations. All 12 questions admitted a control, for 22 passages replaced in each edit condition. Two questions had tied optimal assignments, resolved before decoding. Across questions, the sum of absolute word-count discrepancies between matched passages ranged from 0 to 54. The summed slot distances ranged from 1 to 11. The two edited prompts differed in actual length by 1 to 76 Qwen tokens. [Exact slots and matching discrepancies](manifest.json)

Four reviewed questions admitted alternative plausible answers. For example, the flag question has passages identifying different nations that meet its description, and several characters qualify as ring bearers. An alias miss does not establish an incorrect answer. Unreviewed support, model memory, wording, the shared omission marker and shifted token positions can also affect a response. These 36 greedy generations cost 44.52 seconds on the tested CPU, with no paid calls. This was a controlled follow-up on known questions. The comparison cannot identify what caused the model to use a passage during its original generation. [Passage review and alternate answers](../labels.json)

## Reproduce

A second complete CPU run reproduced all 36 prompt digests, exact output strings, stop tokens and token counts. An independent check against the original answer aliases reproduced each match label.

The saved counts can be verified with Python alone. Supplying the pinned dataset also reconstructs the minimum passage sets and their controls and checks every reference-alias score from the exact generated text. Use the [dataset and CPU setup](../../README.md#repeat-the-model-measurements) for a full rerun.

```sh
python3 examples/public-qa/support-review/witness-edits/generate.py \
  --check examples/public-qa/support-review/witness-edits/answers.json

python3 examples/public-qa/support-review/witness-edits/generate.py \
  --check examples/public-qa/support-review/witness-edits/answers.json --dataset /tmp/a2context-qa.jsonl.gz

python3 examples/public-qa/support-review/witness-edits/generate.py \
  --dataset /tmp/a2context-qa.jsonl.gz --snapshot /path/to/qwen/snapshot --output /tmp/witness-answers.json
```
