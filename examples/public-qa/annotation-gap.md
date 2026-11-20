# Can we detect a missing supporting passage before answering?

On 20 fresh questions, a small Qwen model flagged **one of 20** requests missing their annotated passage. It also flagged the intact version of that same question. A keyword baseline caught one missing passage and raised three false alarms. Neither signal distinguished a missing passage from its intact counterpart often enough to justify a staging alert in this experiment. [Paired scores and thresholds](annotation-gap-results.json)

| Signal | Missing passage flagged | Intact request flagged | Missing version scored higher within pair |
| --- | ---: | ---: | ---: |
| Model probability of the instructed abstention | 1/20 | 1/20 | 8/20 |
| Negative highest passage BM25 score | 1/20 | 3/20 | 8/20 |

Neither method produced a tie. For both methods, every missing request that triggered an alarm also triggered one with the annotated passage restored. The two methods flagged different questions. A reference answer alias remained somewhere in two of the 20 missing requests and appeared in all 20 intact requests. The annotation identifies a specific source. Other passages may still answer the question, and some alias matches may be incidental.

This run used one checkpoint, one instruction and a small, systematically sampled QA corpus. Passage length and downstream token positions can change during substitution. We measured the score for a specified reply before generation. The study did not measure generated answer quality or internal reasoning.

## Design fixed before scoring, 20 November 2026

A retrieval change can return a full list of plausible passages while leaving out the passage that an evaluator expected. The question was whether one score computed from the assembled request could detect that omission in staging.

Use the ten existing pilot questions and twenty new questions at zero-based rows 2000 through 2475 in steps of 25 from the pinned ten-passage QA file. Keep every selected question. Verify that the new question digests also differ from the earlier twenty-question holdout. For each question, reserve the first distractor in dataset order as the replacement. Keep the eight remaining distractors in the same order. Insert either the annotated passage or the reserved distractor into the same slot, cycling through slots 1–9 across questions. Both variants contain nine numbered passages and use the same user question and system instruction. There is no omission marker.

For each request, score the exact instructed abstention `UNKNOWN` followed by the assistant stop token on the pinned CPU Qwen checkpoint. Use teacher forcing in a single forward pass before free generation. Compare with the negative of the largest BM25 passage score, with corpus statistics computed on the nine supplied passages. Each detector sees one request at a time. The question, text and model parameters are its inputs. Evaluation uses dataset annotations and answer aliases.

For each method, use a strict threshold above the second-highest score on the ten support-present pilot requests. This permits at most one pilot false alarm and makes the threshold independent of the twenty evaluation pairs. Report the missing requests flagged, the support-present false alarms, and how often the missing request scores higher than its own control. Count ties separately. Report how often an answer alias still occurs in the supplied passages after the swap. A remaining alias can indicate redundant evidence or an incidental text match.

## Reproduce the measurements

To check the published thresholds and counts from the saved pairs, run `python3 examples/public-qa/annotation_gap.py --check-saved`. To repeat the forward passes, use the dataset and pinned CPU environment from the [public QA setup](README.md#repeat-the-model-measurements). The runner checks the dataset, checkpoint weights, tokenizer and runtime. It records both scores for every input. Repeating the run requires no hosted API.

```sh
python3 examples/public-qa/annotation_gap.py /tmp/nexus-qa.jsonl.gz /path/to/qwen/snapshot \
  --output /tmp/nexus-qa-gap.json
```

Two complete runs on the same CPU produced byte-identical result files. On the first evaluation pair, scoring each token in its own forward pass agreed with the combined score to within 0.000016 natural-log units in both arms.

Liu et al. studied the effect of passage position using this QA corpus. Rajpurkar et al. evaluated abstention on human-written unanswerable questions. Our paired swap tests whether a particular annotated passage disappeared. A workflow that already knows a required document ID can check its presence directly. The harder case here is a plausible retrieval result with no such requirement at runtime. [Lost in the Middle](https://aclanthology.org/2024.tacl-1.9/) · [SQuAD 2.0](https://aclanthology.org/P18-2124/)
