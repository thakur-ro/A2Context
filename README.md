# NexusWeave

**Find which context edits change an LLM’s answer or tool call.**

Ship NexusWeave as a small Rust service beside your LLM workflow. Open the recommendation cards and ask which instructions disagreed, where the problem recurred, or what a source edit changed. Each answer can open the underlying call or replay. [Use the chat canvas with your model](docs/chat-canvas.md)

[![Dashboard walkthrough following a customer's hold instruction from the confirmation cache to the order database](docs/media/exchange-approval.gif)](docs/media/exchange-approval.mp4)

A customer pauses the second of two exchanges. The agent still proposes it after its confirmation cache is refreshed. Ask which instructions disagreed. Then compare what the same proposal did with and without an approval check at dispatch. [Run the walkthrough](docs/demo.md) · [Download the recording](docs/media/exchange-approval.mp4)

## What can I learn?

| Your question | Evidence NexusWeave uses |
| --- | --- |
| Which conversations share a context problem? | Open a recommendation to find the turns with the same declared conflict, repeated input or missing source anchor. [Try a recurring capture gap](docs/demo.md#diagnose-missing-context-capture) |
| Did the request contain the sources my workflow requires? | Before dispatch, check the named fragments and optionally match their text against an upstream digest. [Staging example](docs/guide.md#check-alternative-sources-before-dispatch) |
| Can I edit this passage and isolate one source? | Check which named fragments share the same bytes in the assembled request. [See overlapping edits](docs/guide.md#check-whether-editing-a-passage-touches-two-sources) |
| Which source should I test before answering? | Check the assembled request. If your scorer supplies probabilities, compare candidate continuations before and after a named source edit at the same answer prefix. |
| Would this edit fix the answer elsewhere? | Compare original and edited generations against a supplied reference. The dashboard surfaces matched questions where the same edit had opposite effects on correctness. |

The workflow exports spans asynchronously over OTLP/HTTP. A bounded worker normalizes them and stores findings in SQLite. The Rust binary serves the dashboard. Default analysis makes zero model calls. You can attach replay results or measurements from an instrumented model when you need stronger evidence. A trace records the observable request and response. Internal activation claims require a model intervention. [Algorithms and limits](docs/methods.md) · [Research and related systems](docs/research-review.md) · [What the experiments support](docs/research-preview.md)

## Which source mattered at this word?

![Measured effects of three source edits across a generated answer about returns and refunds](docs/media/source-edits-by-token.png)

On this recorded answer, editing the returns policy had the largest measured effect on the return-window number. Editing the refund policy had the largest effect on the refund time. Each cell compares the probability of the same next token with the preceding output held fixed. Select a cell in the dashboard to see the edit and the two freely generated replies. [Explore the measurements](docs/research-preview.md#follow-a-source-through-a-workflow)

## Try the demo

From the checkout with stable Rust, Make and Python 3.11 or later:

```sh
make demo
```

Open the printed address. The walkthrough uses saved results from local model replays and a sandboxed order database. [Reproduce the measurements](docs/demo.md#what-the-test-covers)

## Ship it beside your service

Send traces from your application's batch exporter or fan out from your existing OpenTelemetry Collector. Keep your existing observability backend. The sidecar listens on loopback within the workflow’s network namespace. Content capture is off by default. [Deployment and Kubernetes example](DEPLOYMENT.md)

```sh
make check          # Exercise the source, UI scripts and detector fixtures
make verify-image   # Build and test the actual container, including container replacement
```

[Full local release check](CONTRIBUTING.md#check-a-release-candidate-on-your-machine) · [Staging checks and resource measurements](docs/validation.md) · [Rust API](docs/rust-api.md) · [Research and dataset credits](CREDITS.md) · [Apache 2.0 license](LICENSE)
