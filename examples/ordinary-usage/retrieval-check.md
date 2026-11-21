# Can earlier requests reveal a missed retrieval opportunity?

Freeze this development experiment before ranking any passages. The five PaperQA sessions already have recorded outcomes. Use them to develop the method. Two known misses concern operating cost (session 2) and incomplete capture (session 4). A later transfer test requires new workflows.

For each of the five questions in arrival order, search only source passages observed in an earlier session from this public corpus. Exclude the current request, its answer, later sessions, generated summaries, and passages already retrieved for the question. Treat this pilot’s four pinned public documents as one explicitly shared collection. Private deployments would need an explicitly authorized corpus and revision boundary before reusing a passage across sessions.

Index the original passage text in SQLite FTS5 with English Porter stemming. Search for the words in the original user question joined by OR. Order by BM25 and break ties by source name. Take the first result, if any. The adapter may read PaperQA’s excerpt boundary to preserve a passage and its exact request location. Require its bytes to occur in the corresponding pinned document. The ranker sees neither the document names nor reviewer labels as search terms.

Report all five candidates and abstentions. Check whether the candidate for session 2 contains measured memory and latency under a stated resource limit. For session 4, check for the score formula and separate assessment coverage. These outcomes evaluate the two previously identified misses. A matching passage supports recommending a retrieval comparison. Whether it changes the final answer requires a separate intervention.
