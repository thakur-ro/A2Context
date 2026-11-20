# NexusWeave

**Catch conflicting instructions. Find repeated context. Test which edits change an LLM’s answer or tool call.**

NexusWeave turns captured workflow inputs and controlled experiments into findings you can trace back to a specific span. It runs alongside your inference workflow and takes telemetry over OpenTelemetry.

![NexusWeave Findings demo showing conflicting returns instructions and baseline versus edited replay trials](docs/media/findings.png)

[Quickstart](#try-the-demo) · [Deploy a sidecar](docs/sidecar.md) · [Methods](docs/methods.md) · [Research review](docs/research-review.md) · [Rust API](docs/rust-api.md)

## Try the demo

From the checkout, run with a stable Rust toolchain and open [Findings](http://127.0.0.1:8040).

```sh
cargo run --locked -- sidecar --demo
```

1. **Catch a conflict.** Open the return-window finding to see the two declared values and the span that carried them.
2. **Find it across sessions.** Filter by organization `demo-retail` and user `user-042`. Open the finding and use **Compare recurrence** with `user-077`.
3. **Test an edit.** Reset the filters, open a verified replay, and select **Open trial comparison**. The demo includes saved local-model trials on synthetic inputs. It makes zero model calls.

## Connect a workflow

Send OTLP/HTTP from a batch exporter in your application or OpenTelemetry Collector. The sidecar processes spans in the background and stores findings in SQLite. Declare stable source IDs, versions, and rule scopes to connect a finding to the original input. [Deploy and configure capture](docs/sidecar.md)

Check an assembled request before serving it with [preflight](docs/guide.md#check-context-before-inference). Use a [controlled replay](docs/guide.md#try-a-decision-replay) to measure how an edit changes a selected outcome. [Read what each finding supports](docs/methods.md).

## Project

NexusWeave uses the [Apache License 2.0](LICENSE). The retail trace excerpt uses [CDLA Permissive 2.0](LICENSES/CDLA-Permissive-2.0.txt). [Contributing](CONTRIBUTING.md) · [Research and dataset credits](CREDITS.md)
