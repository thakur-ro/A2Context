# NexusWeave

**Find the conversations where a context edit changed a model’s answer.**

NexusWeave recommends what to change or test when instructions conflict, an input repeats, or a controlled source edit changes a model’s behavior. Review an assembled request before inference, or let the Rust sidecar surface new evidence as it arrives over OpenTelemetry.

![NexusWeave shows conversation coverage and groups the recommendations by the sources involved](docs/media/findings.png)

[Quickstart](#try-the-demo) · [Deploy a sidecar](docs/sidecar.md) · [Methods](docs/methods.md) · [Research review](docs/research-review.md) · [Rust API](docs/rust-api.md)

## Try the demo

From the checkout, run with a stable Rust toolchain and open [Recommendations](http://127.0.0.1:8040).

```sh
cargo run --locked -- sidecar --demo
```

**Check whether an edit held up.** The first recommendation shows what happened when the runner repeated the same source edit on one input. The proposed tool changed in two of four completed runs. Open **Compare runs** to see the changed and unchanged proposals. [Open the sampled example](http://127.0.0.1:8040/?org=routing-sampling)

**Find which edit changed the answer across conversations.** Select **Generated proposal changed**, then **journal-faq**. Two of the 13 conversations in that category have a changed proposal from a test of the FAQ alone. Open either conversation to compare the original and edited answers. [Explore the edited sources](http://127.0.0.1:8040/?level=1&by=edit)

1. **Check a recommendation before answering.** Editing the dispatch guide changed two earlier answers. On a new request the same edit shifted first-token probabilities, but both answers were `UNKNOWN`. Compare the two answers or download the edit for your own runner. [Open the tested request](http://127.0.0.1:8040/?view=preview&scenario=checked-history)
2. **Check which copy of an instruction mattered.** Choose **Compare the two source orders**. With one replacement, moving a dispatch record from the earlier slot to the later one changed the answer after that record was edited. Try the neutral wording in the same view to see where the match counts held. [Experiment](docs/methods.md#check-the-result-against-new-parcel-codes-and-source-order)
3. **See where the answer depended on a source edit.** Select the refund window in the original answer, then open the edited generation to see it change from four days to seven. [Open the refund example](http://127.0.0.1:8040/?recommendation=6158b8bd5e64d129e9be699f6c78c0965402edc0b7367df671e02ff744171bb1#output-token-14)
4. **Test what to change.** Try a policy conflict before inference. Filter Recommendations by organization `demo-retail` and user `user-042` to compare recurrence with `user-077`.

[Compare four fresh routing requests](http://127.0.0.1:8040/?org=retail-south&factor=5c8549a0e4d873d64b7ffaad88d52f1b33a55c1c3f566abdb43a637e9d906c93&recommendation=fa537f0fdb14f11a458787b85cf6d007e5a02fa1add79ee079258bf3f2578cd9). Two reproduced the earlier edit’s effect on the full response. Only one reproduced its effect on tool selection. Open an example to see the user messages and the trials behind that difference.

The demo uses saved local and hosted model measurements on synthetic inputs and makes zero model calls. To explore public data, compare where the same passage edit changed an answer across two input orders. [Public QA walkthrough](examples/public-qa/README.md#compare-an-edit-across-inputs)

## Connect a workflow

Send OTLP/HTTP from a batch exporter in your application or OpenTelemetry Collector. The sidecar processes spans in the background and stores findings in SQLite. Declare stable source IDs, versions, and rule scopes to connect a finding to the original input. [Deploy and configure capture](docs/sidecar.md)

Check an assembled request before serving it with [preflight](docs/guide.md#check-context-before-inference). Use a [controlled replay](docs/guide.md#try-a-decision-replay) to measure how an edit changes a selected outcome. [Read what each finding supports](docs/methods.md).

## Project

NexusWeave uses the [Apache License 2.0](LICENSE). The retail trace excerpt uses [CDLA Permissive 2.0](LICENSES/CDLA-Permissive-2.0.txt). [Contributing](CONTRIBUTING.md) · [Research and dataset credits](CREDITS.md)
