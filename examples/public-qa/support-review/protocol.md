# Does asking about support detect a missing passage?

The previous staging experiment missed 19 of 20 annotated passage removals. This follow-up fixes one new detector before scoring. It asks the same model whether the supplied passages contain enough information to answer. It measures that judgment before the workflow generates an answer. [Previous experiment](../annotation-gap.md)

## Cases and scoring

Use the ten intact calibration prompts from zero-based rows 0, 100, …, 900 of the pinned dataset. The initial specification requested 12 questions starting at row 2500 in steps of 25. Packet assembly found that the dataset ends at row 2654. Before any scoring or labeling, we kept the seven available rows and filled the shortfall with five interleaved offsets. Evaluate all 12 rows in this order: 2500, 2525, 2550, 2575, 2600, 2625, 2650, 2512, 2537, 2562, 2587, 2612.

For each question, keep eight distractors and insert either the annotated passage or the reserved first distractor at the same position. The position cycles from one to nine within each split. Every condition has nine passages, the same question and no omission marker. Each scorer receives only the question and ordered passage texts. Check that every evaluation question digest differs from the prior pilot and both earlier holdouts.

The two existing alarms retain their prior strict cutoffs of −5.487243987294204 for the log probability of the instructed abstention and −2.1161061263074745 for negative maximum passage BM25. For the new detector, keep the same user message and apply this system instruction exactly.

> Decide whether the supplied passages contain enough evidence to answer the question. Reply YES if the answer can be supported by the passages. Otherwise reply NO. Use only the passages.

At the empty assistant prefix of the pinned Qwen chat template, record the probabilities of the exact single tokens `YES` and `NO`. The gap score is p(NO) / (p(NO) + p(YES)). Also save both full-vocabulary log probabilities so the forced-choice denominator remains visible. Fit a single strict threshold above the second-highest new score on the ten intact calibration prompts. A detector raises an alarm only when its score exceeds its own threshold. Make one forward pass per support judgment and one per evaluation abstention score. The lexical baseline uses zero model calls.

Report the counts of intact and changed prompts flagged, cases in which only the changed version is flagged, and cases where the changed version scores higher or ties the intact version. Measure prompt tokens, input tokens processed, local elapsed scoring time and model calls. Report calibration separately. No free generation and no paid calls. Freeze the independent text-support review before joining the blinded labels to the model scores. The source swap alone establishes that an annotated passage disappeared. Textual sufficiency requires separate evidence.
