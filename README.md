# NexusWeave

**Find which context edits change an LLM’s answer or tool call.**

Ship NexusWeave as a small Rust service beside your LLM workflow. When a policy, retrieved passage, or tool description gives a surprising result, start with the affected conversations. Follow a recommendation to the exact source, the edit someone tested, and the answers it changed.

https://github.com/user-attachments/assets/5553d284-dadc-41a9-9c4f-712b637319ed

A customer pauses the second of two exchanges. The agent still proposes it after its confirmation cache is refreshed. Compare what the proposal did in the sandbox, then see what NexusWeave recommends before the next call. [Run the walkthrough](docs/demo.md)

## What can I learn?

| Your question | Evidence NexusWeave uses |
| --- | --- |
| Which conversations share a context problem? | It groups declared rule conflicts and exact repeated inputs by source, version, and scope. Open a group to read the affected turns. |
| Which source should I test before answering? | Check the assembled request. If your scorer supplies probabilities, compare candidate continuations before and after a named source edit at the same answer prefix. |
| Would this edit fix the answer elsewhere? | Compare original and edited generations against a supplied reference. The dashboard surfaces matched questions where the same edit had opposite effects on correctness. |

The workflow exports spans asynchronously over OTLP/HTTP. A bounded worker normalizes them and stores findings in SQLite. The Rust binary serves the dashboard. Default analysis makes zero model calls. You can attach replay results or measurements from an instrumented model when you need stronger evidence. A trace records the observable request and response. Internal activation claims require a model intervention. [Algorithms and limits](docs/methods.md) · [Research and related systems](docs/research-review.md)

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

[Developer setup](CONTRIBUTING.md) · [Staging checks and resource measurements](docs/validation.md) · [Rust API](docs/rust-api.md) · [Research and dataset credits](CREDITS.md) · [Apache 2.0 license](LICENSE)
