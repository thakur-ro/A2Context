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
