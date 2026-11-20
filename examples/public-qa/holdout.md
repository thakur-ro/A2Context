# Does a pre-inference passage ranking transfer to fresh questions?

## Protocol

We will test the passage selector from the ten-question pilot on 20 further questions from the same pinned public QA file. This protocol fixes the inputs, method and comparisons before generating the new answers.

- Use zero-based rows 1000 through 1950 in steps of 50. Their question texts are distinct and none appears in the pilot. The pilot used rows 0 through 900 in steps of 100. Keep every selected row.
- Run the pilot checkpoint, tokenizer, prompt, omission string, greedy decoding and 48-token limit. Use the original rotation rule at rotation zero. Across these 20 questions the annotated passage occupies each prompt slot twice.
- Before generating any answer, rank the ten passage replacements by Jensen–Shannon divergence of the first-token distributions. Break ties by earlier position. Generate the baseline answer and all ten edited answers.
- The primary comparison is the highest-ranked edit against editing the first passage. Count questions where only the ranked edit changes the exact answer, only the first edit changes it, both change it, or neither changes it. Score a question only if all three required answers finish. Report the paired difference in percentage points and a two-sided exact binomial test on the discordant pairs. A tie on the chosen passage counts as agreement.
- Report BM25, last position and available retrieval rank as descriptive baselines. Also count selection of the dataset’s annotated passage and losses of reference-alias matches. An altered answer can improve or worsen accuracy. Report missing generations separately.
- Record the eleven-forward-pass ranking time separately from decoding. No confidence probability will be assigned to an individual recommendation. One checkpoint, one passage order per question and one retrieval benchmark limit the scope of any result.

## Reproduce

Use the pinned dataset, model snapshot and Python environment from the [public QA study](README.md#repeat-the-model-measurements).

```sh
python3 examples/public-qa/check_passage_edits.py /tmp/nexus-qa.jsonl.gz /path/to/qwen/snapshot \
  --model-id Qwen/Qwen2.5-0.5B-Instruct --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 \
  --cohort holdout --rotation 0 --output /tmp/nexus-qa-holdout.json
```
