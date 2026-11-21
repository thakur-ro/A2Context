# When does removing a retrieved passage leave a question unanswered?

Eight of twelve questions still had a defensible answer in the supplied passages after we replaced the annotated passage. In four, the remaining text supported the reference answer. In four others, it supported a different answer to the question as worded. Three appeared unsupported and one was uncertain. In one case, two passages together gave the end year of martial law by supplying its duration and its start year. A detector trained to flag every annotated removal would penalize that answer. [Reading rubric](annotation.md) · [Passage-level evidence sets](labels.json)

One AI reviewer examined the ten candidate passages for each fresh question with access to the reference aliases. The reviewer could see neither the two request assignments nor the detector scores. We committed the annotations before inspecting the scores. There was no human adjudication. All twelve intact requests supported a reference answer under this review. [Frozen review and protocol](https://github.com/thakur-ro/A2Context/commit/e2f1a1e) · [Exact selection and thresholds](protocol.md)

## Can we warn an engineer before generation?

We asked Qwen2.5-0.5B-Instruct whether the passages supplied enough evidence. The score is the probability of NO conditional on the next token being either YES or NO. We set its threshold on ten intact pilot requests and kept the existing thresholds for the two baselines. Each evaluation request contained nine passages. We counted an alarm as correct if the reviewer could identify no sufficient set. [Per-request scores, token probabilities and timings](scores.json)

| Detector | Unsupported flagged (of 3) | Supported flagged (of 20) | Uncertain flagged (of 1) |
| --- | ---: | ---: | ---: |
| Explicit support question | 1 | 6 | 1 |
| Probability of instructed abstention | 0 | 0 | 0 |
| Negative maximum BM25 | 0 | 0 | 0 |

The support check sounded a new alarm after the edit on just one question, where another plausible answer remained. It missed the loss of the only identified evidence for a filming location. Across the 24 evaluation requests it consumed 33,141 input tokens and 24.06 seconds of CPU-run elapsed time. BM25 took 4.47 milliseconds in total. Two full local runs reproduced the scores. This experiment measured scores before generation. It did not test the answer that the workflow would produce.

## Does the same threshold work elsewhere?

On a separate set of 48 human-labeled SQuAD questions, the same support check flagged 15 of 24 unanswerable questions and one of 24 answerable questions. Each pair shared a single paragraph. The input length, task and reference labels all differed from the retrieval experiment. Both studies used the same checkpoint. With these results we keep the check experimental. [Transfer protocol, confusion matrices and reproduction](transfer/README.md)

## What happened when the model actually answered?

A follow-up generated the original answer, then repeated the request with either supporting passages or an equal number of control passages replaced. Editing supporting passages changed the exact answer text on ten of twelve questions. Control edits changed it on six. One target-only change merely added quotation marks. [Matched edits and generated answers](witness-edits/README.md)

## Reproduce

The first command recomputes calibration, counts and cost from the saved scores. The second joins the frozen review to the exact question and passage digests in both requests and prints every classification. Both work offline with Python 3. To rerun model scoring, use the pinned corpus and checkpoint from the public QA setup. The output path must be unused. [Dataset and CPU setup](../README.md#repeat-the-model-measurements) · [Source attributions](../data-credits.md)

```sh
python3 examples/public-qa/support-review/run.py --check examples/public-qa/support-review/scores.json
python3 examples/public-qa/support-review/compare.py
python3 examples/public-qa/support-review/run.py --dataset /tmp/a2context-qa.jsonl.gz \
  --snapshot /path/to/qwen/snapshot --output /tmp/qa-support-scores.json
python3 examples/public-qa/support-review/compare.py --scores /tmp/qa-support-scores.json
```
