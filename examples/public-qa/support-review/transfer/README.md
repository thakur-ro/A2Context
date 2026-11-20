# Can a pre-answer warning travel between QA workflows?

With thresholds fixed on nine-passage retrieval prompts, asking Qwen whether the evidence was sufficient caught **15 of 24 unanswerable questions** on SQuAD 2.0. It flagged one of the 24 answerable questions. Each question had one paragraph, and each paragraph appeared with a question of each class. The prior retrieval study changed which passages the model saw. Here the paragraph stayed fixed while the question changed. [Selection and scoring protocol](protocol.md) · [Per-question measurements](scores.json)

| Detector | Unanswerable flagged | Answerable flagged | Missed | Correctly unflagged | Sensitivity | Specificity | Unanswerable scored higher within pair |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Explicit support question | 15 | 1 | 9 | 23 | 62.5% | 95.8% | 20/24 |
| Probability of instructed abstention | 8 | 2 | 16 | 22 | 33.3% | 91.7% | 22/24 |
| Negative maximum passage BM25 | 12 | 9 | 12 | 15 | 50.0% | 62.5% | 15/24 |

BM25 tied once. The two model scores had no ties. Ranking and an actionable cutoff behaved differently. The abstention score put the unanswerable question higher in 22 pairs, yet its frozen threshold missed 16 unanswerable questions. A staging workflow usually sees one request and has no paired control to rank it against.

The support question used 48 local CPU forward passes, processing 11,595 input tokens in 7.66 seconds. Abstention took 48 passes and 7.71 seconds for 11,163 input tokens, including one teacher-forced token per request. BM25 took 2.42 milliseconds across all 48 questions. Timings include prompt processing for each scorer and exclude model loading. Both model methods had zero generated completions and used zero API credits.

SQuAD's human authors wrote questions about a supplied Wikipedia paragraph. The benchmark treats answerability as a question about that text. It gives no direct evidence that a model used a particular source or would produce a correct answer. With only one passage, BM25 derives term frequencies from a different-sized corpus than the nine-passage calibration experiment. The balanced subset covers the first 24 eligible titles out of 35. Production prevalence, results for larger models and training exposure to these public examples remain unknown. [Dataset authors and license](https://rajpurkar.github.io/SQuAD-explorer/) · [SQuAD 2.0 paper](https://aclanthology.org/P18-2124/)

## Reproduce

Two full CPU runs produced the same 144 per-question scores and token counts. Selecting questions independently from the presence or absence of human answer spans recovered the same 48 IDs. Recounting threshold crossings with SQL reproduced all three confusion matrices.

Check the saved counts without model dependencies. With the dataset option, the same command also verifies the selected question IDs, ground-truth classes, question digests and paragraph digests against Stanford's development file. The repository contains identifiers and scores. The corpus download stays outside the checkout.

```sh
python3 examples/public-qa/support-review/transfer/squad.py \
  --check examples/public-qa/support-review/transfer/scores.json

curl -fL https://rajpurkar.github.io/SQuAD-explorer/dataset/dev-v2.0.json -o /tmp/squad-dev-v2.json
python3 examples/public-qa/support-review/transfer/squad.py \
  --check examples/public-qa/support-review/transfer/scores.json --dataset /tmp/squad-dev-v2.json
```

The full run uses the [pinned Qwen checkpoint and CPU setup](../../README.md#repeat-the-model-measurements).

```sh
python3 examples/public-qa/support-review/transfer/squad.py --dataset /tmp/squad-dev-v2.json \
  --snapshot /path/to/qwen/snapshot --output /tmp/squad-transfer-scores.json
```
