# The investigation interview

Welcome the user to an all-workflows briefing. Show how many conversations were captured, how many contain findings, and the most common finding types by affected conversation count. Ask **What would you like to investigate?** Offer two or three questions grounded in that briefing. A user can type a different question at any point.

The workspace homepage helps an engineer decide which workflow or problem to investigate. Each workflow homepage shows its own capture stats and one recommended investigation grounded in its highest-ranked finding. Users can start there or type a different question. The workspace can investigate across scopes, including a named workflow.

## How the interview works

| User intent | What appears in the conversation | Evidence it uses |
| --- | --- | --- |
| What needs attention? | A short briefing and a few questions to pursue | Scoped counts and recurring themes |
| What happened here? | An example and the inputs or results relevant to the question | Recorded turns, source locations, and tool observations |
| Compare two workflows | Counts and coverage in one answer, optionally narrowed to one finding type | A scoped summary for each named workflow |
| Find and read a recorded conversation | Up to three chronological calls and a prompt for the next calls | Literal search of captured conversation names and paged stored excerpts |
| Find activity when the title is unknown | Up to four matching excerpts, with a follow-up that shows the supporting call | Full-text search of captured user, assistant, and tool previews |
| Did this happen elsewhere? | A count, a trend when dates are available, and representative conversations | Observations grouped by matching evidence |
| Did a change help? | A comparison and a short explanation of the measured outcome | Original and edited trials, including incomplete evidence |
| Which source mattered at this word? | An interactive token-position diagram when a scored experiment exists | Measured log probabilities for the same output token and preceding output |
| What should I try next? | One suggested check or experiment | The verified limits of the evidence already shown |

The composer stays available through the investigation. Follow-up questions can refer to the previous example. A question about another workflow can scope the answer without leaving the workspace. Supporting calls, conversations, and replay states appear in the interview when requested. Existing saved URLs still resolve.

## Evidence and presentation

The Rust sidecar keeps counts and evidence references authoritative. Without a model, the browser answers a bounded set of question types using those records. A configured model can interpret open-ended questions and choose evidence and views. It works with a bounded catalog of references and cannot supply arbitrary links or overwrite measured values. The A2UI Lit renderer receives the components for each answer. Inferences about source influence need measured evidence under the tested conditions. [A2UI messages](https://a2ui.org/reference/messages/) · [A2UI Basic Catalog](https://a2ui.org/concepts/catalogs/)

System appearance is the default. A light or dark selection persists for the workspace. The interview and linked evidence need readable text, clear focus, and usable layouts on phones and desktops.

Activity search indexes the same bounded excerpts the interview can show. SQLite FTS5 first requires all query terms, then tries any pair or any single term. The interview labels both fallbacks as partial matches. It ranks by BM25 and shows at most one call per conversation, or per trace when no conversation ID was recorded. It uses word matching and English stemming. Capture gaps, excerpt truncation, and paraphrases can prevent a match. A match shows where terms appeared. [SQLite FTS5](https://www.sqlite.org/fts5.html)

## Delivery and checks

1. Ship the briefing, question composer, scoped evidence replies, and theme controls. Keep bookmarked evidence and routes working during the change.
2. Broaden question routing and generated views, including questions about unfamiliar workflows, changes of scope, comparisons across findings, and incomplete evidence. Check a local model and the saved-evidence path against the same questions.
3. Update and record the repository demo. Begin with the briefing, type the first question, and follow the same issue through the source comparison, measured outcome, and recurring cases.
4. Test the flow with the exchange fixture and the full public workload. Verify counts against the API, exact links, time to first answer, keyboard operation, contrast, small screens, unsupported questions, and provider failures. Review where the ranking or answer chose the wrong evidence and rerun those cases after each change.

Browser checks cover the exchange fixture and a separate copy of 18,648 recorded Exgentic requests across 600 conversations and six workflows. The desktop and mobile checks compare the repeat counts for AppWorld and BrowseComp-Plus against the API (74 of 100 and 86 of 100), then open a recorded AppWorld conversation and check the exact call link on its second page. The exchange fixture checks activity with and without captured text. A local Gemma run answered a question about customer verification and cited the recorded tool result. Broader evaluation needs independently labeled questions, particularly ones that require finding relevant evidence across conversations without knowing their titles.

The same sample found airline, AppWorld, and retail activity for three known-term queries entered without workflow names. Browser checks used both desktop and mobile layouts and verified the links against the stored calls. SQLite `dbstat` reported 8.2 MiB of pages for the index and its keys, excluding the write-ahead log. The three queries exercise retrieval and navigation. Estimating recall on unfamiliar questions requires a separate evaluation.
