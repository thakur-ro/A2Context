# Dashboard interview

Select a workflow, ask about a conflict or tool call, and follow the supporting source. The built-in interview can read synthetic demo records without a model connection.

## Connect a model

**Workspace evidence** uses the local sidecar to retrieve findings and recorded excerpts. To interpret the retrieved evidence in plain language, run the optional Python gateway on the same host as the sidecar. It accepts a model server’s Chat Completions endpoint. This example assumes a model already served on loopback. Substitute the sidecar origin printed by the demo and a model ID your server accepts.

```sh
python3 examples/chat-partner/serve.py \
  --sidecar http://127.0.0.1:8040 \
  --endpoint http://127.0.0.1:11434/v1/chat/completions \
  --model YOUR_MODEL_ID
```

Open the gateway address and ask **Was the shopper’s identity confirmed?** in the exchange demo. The partner can select the verification tool by name and show its recorded result. The first configured model is selected automatically. Repeat `--model` to offer several models at one endpoint. For a hosted endpoint, use HTTPS and supply its bearer credential through `A2CONTEXT_CHAT_KEY` in the gateway process environment. The browser never receives the key. Send telemetry and experiment uploads directly to the Rust sidecar.

For an open-ended question, the sidecar searches recorded activity in scope. The partner can choose to read those matches, search different terms or inspect a finding. It sees up to four matching workflow and conversation names during this choice. If it reads activity, a second model request selects lines from the captured calls. The interview shows them verbatim and can open the call that supplied each line. Explicit searches and the token diagram also work with workspace evidence alone. Background analysis makes zero model calls.

The planner receives at most 15 distinct finding descriptions and 16 tool names sampled from the latest 64 scoped calls. For a multi-part question it can also choose from up to 12 combinations of evidence from individual experiments. The browser resolves the selected ID and renders the stored values. Finding interpretation can include recent turns, source names, declared values, state fields, and proposal previews. Activity questions can send excerpts of captured messages and tool results. Choose a provider approved for those data. The gateway allows two concurrent requests and caps each response at 450 tokens. The read selector requests up to 80.

When the retrieved calls leave a question open, the interview says so. Type **Show the retrieved excerpts** to see what it checked. For example, a verification tool can establish that a customer passed the check. Those calls alone cannot establish how the check affected the model’s answer.
