# Can earlier requests reveal a missed retrieval opportunity?

Freeze this development experiment before ranking any passages. The five PaperQA sessions already have recorded outcomes. Use them to develop the method. Two known misses concern operating cost (session 2) and incomplete capture (session 4). A later transfer test requires new workflows.

For each of the five questions in arrival order, search only source passages observed in an earlier session from this public corpus. Exclude the current request, its answer, later sessions, generated summaries, and passages already retrieved for the question. Treat this pilot’s four pinned public documents as one explicitly shared collection. Private deployments would need an explicitly authorized corpus and revision boundary before reusing a passage across sessions.

Index the original passage text in SQLite FTS5 with English Porter stemming. Search for the words in the original user question joined by OR. Order by BM25 and break ties by source name. Take the first result, if any. The adapter may read PaperQA’s excerpt boundary to preserve a passage and its exact request location. Require its bytes to occur in the corresponding pinned document. The ranker sees neither the document names nor reviewer labels as search terms.

Report all five candidates and abstentions. Check whether the candidate for session 2 contains measured memory and latency under a stated resource limit. For session 4, check for the score formula and separate assessment coverage. These outcomes evaluate the two previously identified misses. A matching passage supports recommending a retrieval comparison. Whether it changes the final answer requires a separate intervention.

## Passage search result

The search had no history on question 1. It nominated an earlier passage on each of the four later questions. For both known misses it chose the same passage, foundational-algorithms chunk 2. That chunk contained the score formula and coverage definition. It described what to measure for the deployment budget but supplied no measured cost. The search recovered supporting evidence for one of the two misses. The operating-cost measurements had never appeared in an earlier request at that point. [All five results with exact source locations](retrieval-candidates.json)

## Follow-up registered before generation

Run both known misses through PaperQA’s summarization and final answer generation. Supply the original three captured passages in source-name order for the baseline and its unchanged repeat. For the candidate arm, replace the last of those passages with the top earlier alternative. For a control edit, use the alphabetically first eligible earlier source other than the candidate. Keep the other two passages and the same local model. Set temperature to 0.5, maximum new tokens to 512, concurrency to one and seeds to 11 and 29. Shuffle arm order within each question and seed. No customer workflow or tool is modified.

Score the 16 answers under opaque IDs before revealing their arms. An answer to the budget question must cite measured memory and latency with the deployment limit, consistent with the indexed source. For the incomplete-capture question, it must explain that adding an unresolved eligible run can preserve the assessed pass rate while reducing coverage. Widening bounds can supply additional detail. Record partial answers and unsupported claims. Report all arms including unchanged-repeat disagreements. These are deliberate checks on development data.

**Result.** None of the 16 answers met the registered criterion. All four original/repeat pairs agreed on that judgment. Some budget answers named latency or memory but supplied no measured budget. All eight score answers said incomplete capture decreases the score. We verified the three selected passages, sampling settings and completed response on all 64 underlying model calls. [Blinded review](passage-review.json) · [Arm assignments and captured evidence](passage-arms.json)

On the score question, both candidate arms supplied the correct formula to the summarizer. It returned a general account of pass rates and incomplete data, rating the passage 8/10 and 9/10. Both summaries reached the final model. Neither contained the denominator or the distinction needed to answer the question. This locates where the source detail disappeared in these two runs.

## Follow-up on the summary step

Use each of the two saved candidate-arm final requests for the score question, with its original seed and sampler. Compare four arms in shuffled order: the original, an unchanged repeat, replacing only the candidate summary with its complete verified source passage, and replacing that summary with the first other passage by source-name order captured in the same trial. Use the control passage verbatim and report the character lengths of both replacements. Leave source IDs, wrapper text and the other two summaries fixed. This tests a passage swap in the final input. Only the final generation is rerun. Reuse the score criterion and grade all eight answers with arm IDs concealed. The two seeds belong to one question.

**Result.** All eight requests completed. None met the score criterion. Both unchanged repeats gave the same judgments as their originals. The inserted source passage contained 5,424 characters, compared with 5,464 for the control passage and 364 or 449 for the original summary. Sending the formula verbatim changed both answer texts, yet neither explained that coverage can drop while the assessed pass rate holds. A recommendation to preserve the formula as a fix for this model and question would go beyond the observations. [Blinded answers](summary-review.json) · [Assignments and replacement lengths](summary-arms.json)

To reproduce from a finished public pilot, use Python from the PaperQA environment for the application replay and the same local model server as the pilot. The replay scripts require new output directories.

```sh
python3 examples/ordinary-usage/retrieval_check.py data/ordinary-usage/papers
/tmp/a2-papers/bin/python examples/ordinary-usage/replay_passages.py data/ordinary-usage/papers \
  data/ordinary-usage/passage-comparison --model-port 18742 --binary /path/to/frozen/a2context
python3 examples/ordinary-usage/replay_summary.py data/ordinary-usage/passage-comparison \
  data/ordinary-usage/summary-comparison --model-port 18742
```
