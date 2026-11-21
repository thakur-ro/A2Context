# A2Context

**Find which context edits change an LLM’s answer or tool call.**

A2Context runs beside your LLM workflow. It flags conflicting instructions and missing sources, groups recurring problems across conversations, and shows what happened when someone tested an edit.

**Start here** · [Try the demo](#try-the-demo) · [Check a request before inference](docs/getting-started.md#check-one-request-before-inference) · [Connect your application](docs/getting-started.md#instrument-your-application)

## Try the demo

A customer puts an exchange on hold, but the agent’s confirmation cache still says “approved.” To run this research preview, you need Git, stable Rust and your platform’s C build tools. The demo needs no API key or model download. [Install Rust](https://rustup.rs/)

```sh
git clone https://github.com/thakur-ro/A2Context.git
cd A2Context
cargo run --locked -- demo
```

Open the address printed in the terminal. Ask **What did the customer ask for, and did the approval check change the outcome?** The interview shows the customer’s hold alongside the two recorded outcomes for the puzzle order. Ask **Show the dispatch states** for the sandbox receipt. To follow the original conversation, ask **What did verify_customer return?** and **What happened after that call?** Press Ctrl+C to stop.

[![Dashboard walkthrough following a customer's hold instruction from the confirmation cache to the order database](docs/media/exchange-approval.gif)](docs/media/exchange-approval.mp4)

**Try the receiver.** Send the included exchange batch with curl and watch the two recommendations appear on an empty dashboard. Then connect your application’s exporter to the same endpoint. [Send the sample](docs/getting-started.md#send-the-sample-to-a-running-sidecar) · [Connect your application](docs/getting-started.md#instrument-your-application)

## What can I learn?

| Your question | Evidence A2Context uses |
| --- | --- |
| Did my chat template drop the previous tool call? | Remove one structured field and compare the exact input token IDs before calling the model. [Try the inventory rendering check](examples/local-mlx-tool/rendering/README.md) |
| Can a quote request expose booking tools? | Compare named functions in the assembled catalog with the permissions your workflow supplies. The card shows each mismatch and its location in the request. [Try a tool-scope check](docs/guide.md#check-the-advertised-tools-before-inference) |
| What repeats across one user’s conversations? | Ask for their recurring themes, then choose a chart row to inspect a recorded example. [Try the six-conversation walkthrough](docs/chat-canvas.md#find-patterns-for-one-user) |
| Did the request contain the sources my workflow requires? | Before dispatch, check the named fragments and optionally match their text against an upstream digest. [Staging example](docs/guide.md#check-alternative-sources-before-dispatch) |
| Can I edit this passage and isolate one source? | Check which named fragments share the same bytes in the assembled request. [See overlapping edits](docs/guide.md#check-whether-editing-a-passage-touches-two-sources) |
| Which source should I test before answering? | Check the assembled request. If your scorer supplies probabilities, compare candidate continuations before and after a named source edit at the same answer prefix. |
| Would this edit fix the answer elsewhere? | Compare original and edited generations against a supplied reference. The dashboard surfaces matched questions where the same edit had opposite effects on correctness. |

[Algorithms and limits](docs/methods.md) · [Research and related systems](docs/research-review.md) · [What the experiments support](docs/research-preview.md)

## Which source mattered at this word?

![Measured effects of three source edits across a generated answer about returns and refunds](docs/media/source-edits-by-token.png)

From **All workflows**, ask **Which words were sensitive to the refund policy in Returns demo?** Then ask **What about position 6?** Each cell compares the probability of the same next token with the preceding output held fixed. Select a cell to see the source edit, its measured score, and the freely generated replies. [Explore the measurements](docs/research-preview.md#follow-a-source-through-a-workflow)

## Ship it beside your service

Run the Rust sidecar beside your LLM application. It turns captured inputs into findings you can follow across conversations. It can also check an assembled request before inference and use experiment results to recommend what to test next.

```mermaid
---
config:
  flowchart:
    nodeSpacing: 22
    rankSpacing: 28
    padding: 10
---
flowchart TB
    accTitle: A2Context sidecar and inference paths
    accDescr: The application owns inference and tool execution. It exports captured traces asynchronously and can request a synchronous preview. The sidecar checks captured context and verifies reports from a separate experiment runner. SQLite backs the recommendations dashboard, where an engineer can prepare another test.

    App["LLM application<br/>Runs inference and tools"]
    Runner["Experiment runner<br/>Tests a source edit"]
    Worker["A2Context sidecar<br/>Context and report checks"]
    Store[("SQLite<br/>Evidence and findings")]
    Dashboard["Recommendations<br/>Dashboard and API"]

    App -->|"Async OTLP/HTTP"| Worker
    App <-.->|"Optional preview"| Worker
    Runner -.->|"Replay or probe report"| Worker
    Worker --> Store --> Dashboard
    Dashboard -.->|"Engineer prepares a test"| Runner

    classDef workflow fill:#f0f3f7,stroke:#8293ab,color:#253448
    classDef analysis fill:#e9eff8,stroke:#6b83a5,color:#182c45
    class App,Runner workflow
    class Worker,Store,Dashboard analysis
```

Your application controls inference, authorization, and tool execution. Background analysis makes zero model calls. A preview can flag a missing or conflicting source before dispatch. To measure which source changed a proposed answer or tool call, run a controlled edit separately and attach the report. [Request paths and evidence limits](docs/architecture.md#sidecar-architecture)

Build the container and verify that traces survive a restart with `make verify-image`. A local OpenTelemetry Collector can send the same spans to A2Context and your existing observability backend. [Deployment](DEPLOYMENT.md) · [Collector setup](docs/sidecar.md#fan-out-without-changing-observability-platforms) · [Staging checks and resource measurements](docs/validation.md)

[Study recommendations across six workloads](docs/scale-study.md) · [Contributor setup and release checks](CONTRIBUTING.md) · [Rust API](docs/rust-api.md) · [Connect a model to the dashboard](docs/chat-canvas.md) · [Research and dataset credits](CREDITS.md) · [Apache 2.0 license](LICENSE)
