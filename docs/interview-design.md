# The investigation interview

Welcome the user to an all-workflows briefing. Show how many conversations were captured, how many contain findings, and the most common finding types by affected conversation count. Ask **What would you like to investigate?** Offer two or three questions grounded in that briefing. A user can type a different question at any point.

The workspace homepage helps an engineer decide which workflow or problem to investigate. Each workflow homepage shows its own capture stats and one recommended investigation grounded in its highest-ranked finding. Users can start there or type a different question. The workspace can investigate across scopes, including a named workflow.

Put the question first on the workspace homepage. Each recurrence row is a keyboard-accessible action in the A2UI evidence catalog. Selecting a row and typing the same example request retrieve the top-ranked finding of that kind under the active filters. The host reads the finding and its evidence from SQLite. These interactions make zero model calls. A named user carries through follow-ups, workflow links and reloads. The gallery browser check compares those answers with scoped API records and distinguishes a user with zero findings from an ID with no recorded conversations. [User walkthrough](chat-canvas.md#find-patterns-for-one-user) · [Browser check](../tests/interview-user-browser.mjs)

## How the interview works

| User intent | What appears in the conversation | Evidence it uses |
| --- | --- | --- |
| What needs attention? | A short briefing and a few questions to pursue | Scoped counts and recurring themes |
| What happened here? | An example and the inputs or results relevant to the question | Recorded turns, source locations, and tool observations |
| Compare two workflows | Counts and coverage in one answer, optionally narrowed to one finding type | A scoped summary for each named workflow |
| Find and read a recorded conversation | Up to three chronological calls and a prompt for the next calls | Literal search of captured conversation names and paged stored excerpts |
| Find activity when the title is unknown | Up to four matching excerpts, with a follow-up that shows the supporting call | Full-text search of captured user, assistant, and tool previews |
| What happened before or after this call? | The neighboring call, its position and captured dialogue | A scoped read anchored to the selected trace and call IDs |
| Did this happen elsewhere? | A count, a trend when dates are available, and representative conversations | Observations grouped by matching evidence |
| Did a change help? | A comparison and a short explanation of the measured outcome | Original and edited trials, including incomplete evidence |
| What did the user ask for, and did a check change the outcome? | The last captured user message and both sandbox outcomes in the same answer | The message preview at the replay’s trace and call IDs, and the verified dispatch receipt |
| Which source mattered at this word? | An interactive token-position diagram when a scored experiment exists | Measured log probabilities for the same output token and preceding output |
| What should I try next? | One suggested check or experiment | The verified limits of the evidence already shown |

The composer stays available through the investigation. Follow-up questions can refer to the previous example. A question about another workflow can scope the answer without leaving the workspace. Supporting calls, conversations, and replay states appear in the interview when requested. Existing saved URLs still resolve.

We checked the workspace with a newly recorded observatory workflow. Two sites had different dome reopening times, while an unrelated workflow contained the only conflict. Local Gemma initially selected that conflict. The planner now sees the names of matching conversations before it chooses a read. Asked when Kestrel Ridge reopened, it selected the assistant’s recorded “03:41 UTC” line. Typing “What supports that?” opened the originating call. This tests retrieval and source identity on one unfamiliar topic. [Planner regression](../tests/chat-canvas.test.mjs) · [Cross-workflow browser checks](../scripts/check_conversation_drilldown.mjs)

A numbered search result identifies an example. Its number can differ from the call’s position in the recording. When someone asks what came before or after an example, the interview locates its trace and call IDs and reads the neighbor in the same database snapshot. It asks which call to use when several are visible. If a conversation ID is missing, it can follow calls within the same trace. Missing or tied timestamps are labeled. Recorded order alone cannot establish that an earlier event caused a later decision.

## Evidence and presentation

### Following a chart into a conversation

In the exchange demo, the interview used to list two conversations with a conflict and then answer “Open the second one” with a search result from a third conversation. The displayed choices now carry their conversation IDs. For a finding, each choice also carries its trace and call IDs. Typing an ordinal or an exact title uses the same reference as selecting the row. A connected model can choose a listed conversation by its short-lived ID. The model receives the title and position. The host supplies the filters and resolves the stored IDs.

A chart counts distinct conversations across all finding groups of the given kind. A single finding has its own, narrower set of occurrences. The interview queries the relevant set, returns one matching call per identified conversation and pages three at a time. A count shared by two chart rows prompts the user to choose a theme. A reference to “the second one” uses the order on the page currently in view.

We tested this on the six-workflow Exgentic sample. In AppWorld, 74 conversations contained 3,300 distinct repeated-input groups. Across all six workflows, the browser matched the first two pages to the sidecar, opened the second conversation on page two, and checked its trace and call IDs against the stored occurrence. The gallery test covers tied counts, missing user IDs and named-user scope at two viewport widths. In a separate exchange-demo run, local Gemma resolved “the one whose title asks to hold the jigsaw” to **Hold the puzzle** and opened call 6. That run checks one paraphrase. [Public sample](scale-study.md#import-the-public-corpus) · [Six-workflow check](../scripts/check_conversation_drilldown.mjs) · [Gallery check](../tests/interview-user-browser.mjs)

Run the public-sample check against a sidecar serving the imported recordings. It needs Playwright and Chromium.

```sh
node scripts/check_conversation_drilldown.mjs http://127.0.0.1:18052
```

### Asking what to test next

“What should I test next?” uses the selected finding. When the answer shows calls, the interview can look up a finding at one of those exact calls. If none has a finding, it says so. From the workspace briefing the same question uses the highest-ranked finding in scope.

For a question that names a topic, workspace evidence ranks available recommendations by the number of distinct topic terms that match their source names, tool, workflow, finding, observation, example or proposed check. Ties keep the incoming finding order. A connected model can choose among up to five matching recommendations, or return no match. The host displays the stored next action and its supporting observation. Retrieval can miss a paraphrase that shares no terms with the available findings, and a shared term alone does not establish relevance.

In the exchange fixture, asking what to test before another order update originally returned excerpts or a comparison of old model outputs. Workspace evidence and local Gemma now show the suggested valid-approval control and the sandbox outcome that motivated it. The control remains a proposed experiment. The same checks cover a refund-policy token experiment, an unrelated topic, and a recorded conversation without findings. [Exchange walkthrough](demo.md#run-the-investigation) · [Browser regression](../tests/exchange-a2ui-browser.mjs)

### Renderer

A2Context ships the A2UI 1.0 protocol with the Lit renderer and web core pinned to 0.15.0. The protocol describes each surface. The host chooses the layout, type, color and keyboard behavior. The recurrence chart is a custom catalog component with a bounded action that selects a recorded finding. Its counts describe kinds of evidence. For example, a proposal trial records a comparison, and a rule conflict records a disagreement in the inputs. [A2UI 1.0 release](https://developers.googleblog.com/a2ui-10-build-delightful-fast-reliable-agentic-experiences/) · [Renderer package pins](../ui/a2ui/package.json)

The Rust sidecar keeps counts and evidence references authoritative. Without a model, the browser answers a bounded set of question types using those records. A configured model can interpret open-ended questions and choose evidence and views. It works with a bounded catalog of references and cannot supply arbitrary links or overwrite measured values. The A2UI Lit renderer receives the components for each answer. Inferences about source influence need measured evidence under the tested conditions. [A2UI messages](https://a2ui.org/reference/messages/) · [A2UI Basic Catalog](https://a2ui.org/concepts/catalogs/)

The workspace shows the greeting, counts, question composer, suggested prompts, and a frequency chart. Each bar uses all captured conversations in scope as its denominator. The workflow home highlights its recommended investigation. Asking which issues keep coming up renders the chart again for the named workflow.

A replay can answer several parts of a question. In the exchange fixture, editing the confirmation cache left the model’s proposal unchanged. Applying an approval check at dispatch changed the sandbox order state. The interview can show either result, both results, or the user’s recorded message alongside them. All sections refer to one experiment. A missing message or measurement is labeled in place. When multiple experiments tie on the supplied names, the engineer can choose by typing an experiment number.

System appearance is the default. A light or dark selection persists for the workspace. The interview and linked evidence need readable text, clear focus, and usable layouts on phones and desktops.

Activity search indexes the same bounded excerpts the interview can show. SQLite FTS5 first requires all query terms, then tries any pair or any single term. The interview labels both fallbacks as partial matches. It ranks by BM25 and shows at most one call per conversation, or per trace when no conversation ID was recorded. It uses word matching and English stemming. Capture gaps, excerpt truncation, and paraphrases can prevent a match. A match shows where terms appeared. [SQLite FTS5](https://www.sqlite.org/fts5.html)

With a model connected, an engineer can describe the task without knowing a tool or chart name. The partner can select a scoped read by ID or rewrite the question into two short searches. Each rewrite requires all its terms in the same recorded excerpt. The browser interleaves exact results and preserves the call references. For event questions the model can select recorded lines, and the interview quotes them with their roles. An assistant reply about a refund cannot establish which words depended on the refund policy. That question needs the scored intervention. Query reformulation here adapts the retrieval step of Ma et al. Their evaluation used other tasks and retrieval systems. [Query Rewriting in Retrieval-Augmented Large Language Models](https://aclanthology.org/2023.emnlp-main.322/)

## Delivery and checks

1. Ship the briefing, question composer, scoped evidence replies, and theme controls. Keep bookmarked evidence and routes working during the change.
2. Broaden question routing and generated views, including questions about unfamiliar workflows, changes of scope, comparisons across findings, and incomplete evidence. Check a local model and the saved-evidence path against the same questions.
3. Update and record the repository demo. Begin with the briefing, type the first question, and follow the same issue through the source comparison, measured outcome, and recurring cases.
4. Test the flow with the exchange fixture and the full public workload. Verify counts against the API, exact links, time to first answer, keyboard operation, contrast, small screens, unsupported questions, and provider failures. Review where the ranking or answer chose the wrong evidence and rerun those cases after each change.

The call-order check opens one conversation from each of the six Exgentic workflows through the all-workflows composer. It checks the preceding and following calls against their stored trace and call IDs. The exchange fixture covers the first call, ambiguous references, and asking a follow-up without leaving the homepage.

Browser checks cover the exchange fixture and a separate copy of 18,648 recorded Exgentic requests across 600 conversations and six workflows. The desktop and mobile checks compare the repeat counts for AppWorld and BrowseComp-Plus against the API (74 of 100 and 86 of 100), then open a recorded AppWorld conversation and check the exact call link on its second page. The exchange fixture checks activity with and without captured text. A local Gemma run answered a question about customer verification and cited the recorded tool result. Broader evaluation needs independently labeled questions, particularly ones that require finding relevant evidence across conversations without knowing their titles.

The same sample found airline, AppWorld, and retail activity for three known-term queries entered without workflow names. Browser checks used both desktop and mobile layouts and verified the links against the stored calls. SQLite `dbstat` reported 8.2 MiB of pages for the index and its keys, excluding the write-ahead log. The three queries exercise retrieval and navigation. Estimating recall on unfamiliar questions requires a separate evaluation.

### More than one kind of evidence in an answer

Seven questions cover the saved retail exchange and telecom refueling experiments. Three ask for multiple pieces of evidence. The checker opens each question from the workspace, matches the trace, call and report IDs, and compares the displayed messages, proposals and state values with the saved data. All seven passed with workspace evidence and with the local model connected. The named experiments and fields in these questions let the host resolve all seven without a model request. A separate Chrome check asks the user to choose between two equally matched experiments and verifies that a redacted message stays unavailable while its recorded outcome remains visible.

Run the check with the measured telecom packet from the freshness study. The script adds the exchange fixture to a temporary sidecar. Pass `--endpoint` and `--model` to repeat with a connected partner. [Generate the telecom packet](recommendation-loop.md) · [Replay check](../scripts/check_interview_replays.py)

```sh
python3 scripts/check_interview_replays.py /path/to/telecom-refuel-measured.json
```

### Questions phrased in the engineer’s own words

Five development questions check whether the first excerpt contains evidence for the requested activity, or whether the interview renders the measured diagram. The script checks content and the recorded workflow where applicable. It also opens the selected call and resolves its ID against the sidecar. Two runs on this machine produced the same results.

| Question | Workspace evidence | With local Gemma 3 4B |
| --- | --- | --- |
| Was the shopper’s identity confirmed? | Missed the tool result | Quoted the verification result |
| Show requests to undo a plane booking. | Missed | Missed |
| What peer-to-peer cash movements did people ask for in Venmo? | Found related activity | Found related activity |
| Where did shoppers ask to swap a jigsaw? | Missed | Found an exchange involving a puzzle |
| Which words were most sensitive to changing the refund instructions? | Showed the measurement | Showed the measurement |

The airline sample illustrates the difference between finding a topic and answering a question. Cancellation policy and the agent’s account of a request appeared in the retrieved excerpts. The first excerpt did not establish a customer’s request. A read can search only what the exporter captured and the index retained. The five prompts informed development. They cannot estimate accuracy on new workflows.

We then wrote five fresh questions and ran them before changing the router. Both modes met three of the five evidence checks. Both found a verification result and telecom activity about internet use abroad. Gemma selected the token experiment for a question about the numbers in an answer. Its rewrite missed the relevant software-test excerpt that workspace search found. Both missed the dispatch receipt when asked for the order database under two versions of an approval check. These checks score the evidence shown first. Finding a verification result alone does not establish when verification happened relative to another event.

On the redesigned dashboard at `7acd93d`, we repeated those five questions. Workspace evidence opened the expected evidence for four. It missed the token experiment when the question described “the numbers in the response.” Local Gemma 3 4B opened the expected evidence for all five. These are now regression checks because the team has seen the questions. They measure which evidence appeared first. We have yet to test whether engineers understand that evidence more quickly with the new layout.

To repeat the comparison, run a model gateway for the exchange demo and a second gateway for the imported six-workflow sample. Pass their origins and the configured model ID to the browser check. Each JSON line includes the wording, whether the content check matched, the searches issued, and the number of model calls. [Build the public sample](scale-study.md#import-the-public-corpus) · [Configure the gateway](chat-canvas.md#connect-a-model)

```sh
node scripts/check_interview_questions.mjs "$DEMO_GATEWAY" "$SAMPLE_GATEWAY" "$MODEL_ID"
node scripts/check_interview_questions.mjs "$DEMO_GATEWAY" "$SAMPLE_GATEWAY" "$MODEL_ID" transfer
```
