# Can an engineer find something worth fixing during ordinary use?

This is a prospective development pilot with two public applications. The reviewer is the A2Context project owner. The reviewer did not write either application. A review can establish whether an engineer would investigate the evidence. A production incident or a fix confirmed by the application maintainer needs separate evidence.

## Frozen before the first request

Use A2Context revision `efc533203cfae310dc4fde57fdcf1ac60295dd98`, its default ranking, content capture enabled on loopback, an empty database per workflow and no optional model-assisted analysis. Keep the same binary for every run. Record each provider request as sent, the response when available, and any capture gaps. Keep transport failures and application exits in the run log. A provider response alone cannot establish what the end user saw.

| Public application | Actual work | Inputs |
| --- | --- | --- |
| Simon Willison’s LLM 0.38 with llm-tools-sqlite 0.1 | Use the read-only SQLite agent to investigate A2Context’s development history for release planning. | Commit dates, subjects and changed paths from this public repository through the frozen revision. |
| FutureHouse PaperQA 2026.8.12 | Use its document retrieval and answer workflow to research the evidence behind the release claims. | The frozen repository’s methods, validation, foundational algorithms and research review documents. Supply their titles at indexing. |

The applications have separate implementations and dependencies. Use a local instruction model and record its identity. The tool workflow can inspect the database schema. PaperQA can search and summarize the documents. Preserve the applications’ normal prompts, tools and outputs. Neither application needs expected answers, incident labels or custom detection rules. [SQLite agent](https://github.com/simonw/llm-tools-sqlite) · [PaperQA](https://github.com/Future-House/paper-qa)

## Run order and stopping rule

Ask these questions once in order, in separate sessions. These are outstanding engineering questions. There is no expected result for any question.

| Order | Repository history | Research evidence |
| --- | --- | --- |
| 1 | Which parts of the sidecar have changed most often recently? | Which evidence can establish exactly what an application sent to a model? |
| 2 | When did the project introduce retention and redaction, and which files changed? | Which published measurements support the proposed operating budget? |
| 3 | Which UI changes landed alongside browser tests? Give me three commits to inspect. | How does the project distinguish a source conflict from an effect on the answer? |
| 4 | Which recent changes touched both the database schema and its migration tests? | What happens to the workflow score when capture is incomplete? |
| 5 | Which deployment and onboarding files have had the most changes since this project began? | Which comparisons against related systems still need an independent test? |

Allow five minutes per question. Preserve partial runs and count unanswered questions. Do not introduce a fault, change a prompt after seeing an answer, replace a quiet workflow, or extend the pilot until five cards appear. Record connection time, the time each distinct recommendation first appears, the exact evidence reference and the review time in UTC. Keep the first five by first appearance, breaking simultaneous ties by the default feed order. Repeated occurrences of one recommendation occupy one place.

The reviewer marks each suggestion worth investigating, dismissed with reason, or unreviewed. A useful suggestion identifies a concrete next step backed by the linked evidence. Also review up to three quiet runs per workflow, taking the lowest SHA-256 hashes of their trace IDs in UTF-8. Report missed issues separately. If fewer than five distinct suggestions appear, report the empty slots and consider the release gate unmet. Measure elapsed time to the first engineer-confirmed useful suggestion including review wait. Report time to machine visibility separately.
