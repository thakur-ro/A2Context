# Ask about a workflow

Open the sidecar dashboard to see the workspace briefing. It shows captured conversations, affected conversations, and recurring themes. Pick a suggested question or type your own. Choose a workflow in the sidebar for its briefing, or type **Focus on** followed by its name. Type **Show all workflows** to return to the workspace. A conversation can contribute to several finding groups.

Ask “Which instructions disagreed?” to compare the values declared by two sources. Ask “Show the exact inputs” for the captured excerpts behind that answer, or “Where else did that repeat?” for counts and representative conversations. A sandbox receipt opens as a diagram with the same proposal on both branches. **Open exact evidence** takes you to the recorded call. The appearance control switches between light and dark and remembers the setting on that origin. [Interview design](interview-design.md)

From the workspace briefing, type **Compare** followed by two workflow names. The answer shows captured conversations with findings, the denominator, and the most common finding type. Add **by repeated inputs**, **rule conflicts**, or **source capture gaps** to compare one class. The percentages describe captured evidence. Different workflows can record different amounts of evidence.

Ask **Show recent conversations in** followed by a workflow name to find activity from the workspace homepage. Type **Find conversation** followed by part of its recorded name. A unique match opens up to three captured calls in the interview, including recorded tool results when available. Type **What happened next?** to read the next calls or ask **What did the tool return?** for the visible results. This works for conversations without a finding. When a model is connected, it can interpret the calls in view and cite them. Each evidence link comes from the store.

## Connect a model

**Workspace evidence** runs in the browser and uses word matching and a small set of question types. For free-form questions, run the optional Python gateway on the same host as the sidecar. It accepts a model server’s Chat Completions endpoint. This example assumes a model already served on loopback. Substitute the sidecar origin printed by the demo and a model ID your server accepts.

```sh
python3 examples/chat-partner/serve.py \
  --sidecar http://127.0.0.1:8040 \
  --endpoint http://127.0.0.1:11434/v1/chat/completions \
  --model YOUR_MODEL_ID
```

Open the gateway address and ask a question. The first configured model is selected automatically. Repeat `--model` to offer several models at one endpoint. For a hosted endpoint, use HTTPS and supply its bearer credential through `NEXUSWEAVE_CHAT_KEY` in the gateway process environment. The browser never receives the key. Send telemetry and experiment uploads directly to the Rust sidecar.

A question sent to a partner makes one request. A direct question about a uniquely matched dispatch comparison reads the sandbox receipt and makes zero model calls. An open-ended question about findings includes the current question, up to six recent turns, and up to 15 ranked findings. When the scope is larger, the sidecar also searches source names, rules, tool names, finding types and conversation names using up to eight terms from the question. Summaries can include source names, declared values, state fields and proposal previews. A question about a conversation can send the message and tool result excerpts for the calls in view. Choose a provider approved for that data. The gateway caps the response at 450 tokens and allows two concurrent partner requests. Workspace evidence, background analysis and opening cards make zero model calls.

## How an answer becomes a diagram

1. The Rust sidecar groups observations in SQLite and returns the ranked findings and their exact evidence references.
2. When a question identifies one evidence type with a single matching record, the canvas sends that record to the partner for a plain-language answer. For a measured dispatch comparison it reads the two target states directly from the receipt. For a recurrence follow-up, the partner receives the finding name, counts, and an example conversation. For broader questions the canvas gives the model short references to the retrieved findings. The model returns prose, one reference, and a requested view. A reference outside the supplied set produces a visible error. A recognized reference with an unfamiliar view opens its evidence card.
3. The browser sends successive A2UI v1.0 component updates to one Lit surface. The heading appears first, followed by the evidence and a follow-up action. A comparison draws both arms together. Selecting a declaration shows the captured role and version. Selecting a sandbox result shows its dispatch status. Choosing another question supersedes the current answer. Reduced-motion preferences show the available components immediately. The sidecar supplies the records, and the browser composes the layout. A model cannot supply a destination URL or replace a measured value. A rule disagreement shows captured declarations. A replay diagram compares the tested conditions. The partner’s prose is an interpretation.

**Choose different evidence** corrects a match for the same question on the open page. Refreshing or switching scope clears that correction and the conversation. This lets an engineer recover a bad match without changing ranking for other users.

The browser regression replays the exchange questions, checks the linked call and report, and checks that a corrected question opens the selected evidence. It also checks a query with no matching evidence. The exchange fixture exercises this path on two affected conversations. Broader claims about routing accuracy require a separately labeled question set. [Exchange walkthrough](demo.md)

## Check a partner before using it

Run the browser test against the exchange demo with the gateway enabled. Supply the names from your gateway and Playwright installation.

```sh
CHAT_MODEL=YOUR_MODEL_ID node tests/exchange-a2ui-browser.mjs http://127.0.0.1:8041
```

The optional live check asks three questions and makes two model requests. It asks which source supplied the hold, what changed at dispatch, and where that result repeated. It checks the selected evidence and the rendered conversation link. An engineer should also read the generated prose. The graph can be correct even when a model describes it poorly.

In the walkthrough, Gemma 3 4B identified the customer message as the source of the hold. In development, Qwen2.5 0.5B returned a valid finding ID with an unfamiliar view name on one question and carried an earlier conflict into a later dispatch question. Gemma also described two sandbox arms as a chronological transition. The dispatch answer now names each arm from the receipt. Those cases led us to keep chart values in the renderer and narrow single-match questions to the matching evidence before generation. These observations come from the saved exchange case.
