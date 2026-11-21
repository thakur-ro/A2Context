# Ask about a workflow

Open the sidecar dashboard to see workspace totals and recurring themes. Pick a suggested question or type your own. Choose a workflow in the sidebar for its stats and recommended investigation, or type **Focus on** followed by its name. Type **Show all workflows** to return to the workspace. A conversation can contribute to several finding groups.

Ask “Which instructions disagreed?” to compare the values declared by two sources. Ask “Show the exact inputs” for the captured excerpts behind that answer, or “Where else did that repeat?” for counts and representative conversations. A sandbox comparison shows both branches of the same proposal. Ask “Show the order states” for the saved receipt. The appearance control switches between light and dark and remembers the setting on that origin. [Interview design](interview-design.md)

From the workspace briefing, type **Compare** followed by two workflow names. The answer shows captured conversations with findings, the denominator, and the most common finding type. Add **by repeated inputs**, **rule conflicts**, or **source capture gaps** to compare one class. The percentages describe captured evidence. Different workflows can record different amounts of evidence.

Ask **Show recent conversations in** followed by a workflow name to find activity from the workspace homepage. Type **Find conversation** followed by part of its recorded name. A unique match opens up to three captured calls in the interview, including recorded tool results when available. Type **What happened next?** to read the next calls or ask **What did the tool return?** for the visible results. This works for conversations without a finding. When a model is connected, it can interpret the calls in view and cite them. Ask for a call by number to see the stored record.

When you don’t know the conversation name, ask about a recorded phrase, entity, or tool. In the exchange demo, try **What did verify_customer return?** from the workspace homepage. Ask **Show call 1** to see the supporting record inline. **Search for** followed by a few words searches captured activity. Add a workflow name after **in** to narrow the search.

## Connect a model

**Workspace evidence** uses the local sidecar to retrieve findings and recorded excerpts. To interpret the retrieved evidence in plain language, run the optional Python gateway on the same host as the sidecar. It accepts a model server’s Chat Completions endpoint. This example assumes a model already served on loopback. Substitute the sidecar origin printed by the demo and a model ID your server accepts.

```sh
python3 examples/chat-partner/serve.py \
  --sidecar http://127.0.0.1:8040 \
  --endpoint http://127.0.0.1:11434/v1/chat/completions \
  --model YOUR_MODEL_ID
```

Open the gateway address and ask **Was the shopper’s identity confirmed?** in the exchange demo. The partner can select the verification tool by name and show its recorded result. Ask **Which words were most sensitive to changing the refund instructions?** to see the saved experiment. The first configured model is selected automatically. Repeat `--model` to offer several models at one endpoint. For a hosted endpoint, use HTTPS and supply its bearer credential through `NEXUSWEAVE_CHAT_KEY` in the gateway process environment. The browser never receives the key. Send telemetry and experiment uploads directly to the Rust sidecar.

An open-ended question can make two model requests. First the partner chooses a read using the question, the preceding question when available, and a short list of operations derived from findings and recently recorded tool names. If the read retrieves activity, the partner can select individual lines from up to four calls. The interview displays those lines verbatim, including whether a tool was proposed or returned a result. The sidecar handles explicit searches, and a request for the token diagram reads the stored scores directly. Workspace evidence and background analysis make zero model calls.

The planner receives at most 15 distinct finding descriptions and 16 tool names sampled from the latest 64 scoped calls. Finding interpretation can include recent turns, source names, declared values, state fields, and proposal previews. Activity questions can send excerpts of captured messages and tool results. Choose a provider approved for those data. The gateway allows two concurrent requests and caps each response at 450 tokens. The read selector requests up to 80.

When the retrieved calls leave a question open, the interview says so. Type **Show the retrieved excerpts** to see what it checked. For example, a verification tool can establish that a customer passed the check. Those calls alone cannot establish how the check affected the model’s answer.

## How an answer becomes a diagram

1. The Rust sidecar groups observations in SQLite and returns the ranked findings and their exact evidence references.
2. For an open question, each available read has an ID. A read can retrieve executions or proposals for a named tool, display a measured comparison, or return to the evidence in the last answer. The host maps the ID to the stored evidence and view. The partner can also suggest two short searches. Each alternative must match all its words in one recorded excerpt. If neither matches, the sidecar can show partial matches to the original wording. A question about tool activity can only quote lines from the retrieved sample. A uniquely identified finding can also be interpreted in plain language.
3. The browser sends successive A2UI v1.0 component updates to one Lit surface. The heading appears first, followed by the evidence and a follow-up action. A comparison draws both arms together. Selecting a declaration shows the captured role and version. Selecting a sandbox result shows its dispatch status. Choosing another question supersedes the current answer. Reduced-motion preferences show the available components immediately. The sidecar supplies the records, and the browser composes the layout. A model cannot supply a destination URL or replace a measured value. A rule disagreement shows captured declarations. A replay diagram compares the tested conditions. The partner’s prose is an interpretation.

**Choose different evidence** corrects a match for the same question on the open page. Refreshing or switching scope clears that correction and the conversation. This lets an engineer recover a bad match without changing ranking for other users.

The browser regression checks that supporting calls and the saved comparison appear in the interview and match the stored evidence. It also checks a query with no matches, a workflow without token measurements, and the measured returns diagram. Routing accuracy on other questions still needs a separately labeled question set. [Exchange walkthrough](demo.md)

## Check a partner before using it

Run the browser test against the exchange demo with the gateway enabled. Supply the names from your gateway and Playwright installation.

```sh
CHAT_MODEL=YOUR_MODEL_ID node tests/exchange-a2ui-browser.mjs http://127.0.0.1:8041
```

The optional live check asks whether the shopper was verified, what changed at dispatch, and where the result repeated. The browser also exercises a fixed read selection to check that a follow-up uses the same captured result and that an unrelated question can switch to a measured comparison.

In the walkthrough, Gemma 3 4B identified the customer message as the source of the hold. In development, Qwen2.5 0.5B returned a valid finding ID with an unfamiliar view name on one question and carried an earlier conflict into a later dispatch question. Gemma also described two sandbox arms as a chronological transition. The dispatch answer now names each arm from the receipt. Those cases led us to keep chart values in the renderer and narrow single-match questions to the matching evidence before generation. These observations come from the saved exchange case.
