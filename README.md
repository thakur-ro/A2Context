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

**Read a conversation with a conflicting answer.** Follow the jacket question from the proposed order lookup to the 60-day answer. The finding on that call opens the two sources that disagree. [Open the conversation](http://127.0.0.1:8040/?view=threads&thread=%5B%22fixture%22%2C%22Northland+Retail%22%2C%22Customer+042%22%2C%22Return+a+jacket%22%5D) · [Try eight public retail sessions](docs/sidecar.md#read-a-conversation-turn-by-turn)

**Check whether an edit held up.** With the original instruction and both tools available, the proposed tool changed in two of four runs on the same input. Open **Compare runs** for the repeats, or expand **Answers or tool calls compared** to find changed and matching proposals across conversations. [Compare two setups](http://127.0.0.1:8040/?org=routing-sampling&proposal_result=both)

**Find out why a proposed fix needs another look.** Three return questions missed the exact answer `45`. In two, another opening token led under every tested edit. In the third, one edit produced `45 days`. Open the first group to compare token scores or the second to inspect the extra text. [Find recurring misses](http://127.0.0.1:8040/?org=journal-window-study&targets=1) · [Inspect scores before generation](http://127.0.0.1:8040/?view=preview&scenario=window-before) · [See the fresh question](http://127.0.0.1:8040/?view=preview&scenario=window-fresh)

1. **Check a recommendation before answering.** Editing the dispatch guide changed two earlier answers. On a new request the same edit shifted first-token probabilities, but both answers were `UNKNOWN`. Compare the two answers or download the edit for your own runner. [Open the tested request](http://127.0.0.1:8040/?view=preview&scenario=checked-history)
2. **Check which copy of an instruction mattered.** Choose **Compare the two source orders**. With one replacement, moving a dispatch record from the earlier slot to the later one changed the answer after that record was edited. Try the neutral wording in the same view to see where the match counts held. [Experiment](docs/methods.md#check-the-result-against-new-parcel-codes-and-source-order)
3. **See where the answer depended on a source edit.** Select the refund window in the original answer, then open the edited generation to see it change from four days to seven. [Open the refund example](http://127.0.0.1:8040/?recommendation=6158b8bd5e64d129e9be699f6c78c0965402edc0b7367df671e02ff744171bb1#output-token-14)
4. **Find a recurring instruction conflict.** The journal policy says 30 days and the FAQ says 60. Open the flagged pair to see which source edit changed each answer, read the user’s question, and find the call still awaiting a test. [Compare the conversations](http://127.0.0.1:8040/?screens=1) · [See which edits reached a 45-day target](http://127.0.0.1:8040/?view=preview&scenario=window-earlier)

[Compare four fresh routing requests](http://127.0.0.1:8040/?org=retail-south&factor=5c8549a0e4d873d64b7ffaad88d52f1b33a55c1c3f566abdb43a637e9d906c93&recommendation=fa537f0fdb14f11a458787b85cf6d007e5a02fa1add79ee079258bf3f2578cd9). Two reproduced the earlier edit’s effect on the full response. Only one reproduced its effect on tool selection. Open an example to see the user messages and the trials behind that difference.

The demo uses saved measurements and makes zero model calls. On 20 held-out public QA questions, two lower-ranked passage edits changed answers that the top edit preserved. Follow one from the recommendation to the token where the answers diverged. A separate reference check flags three questions where removing a passage lost an accepted answer. [Explore the public QA study](examples/public-qa/holdout.md#when-did-a-lower-ranked-edit-change-the-answer)

## Connect a workflow

Send OTLP/HTTP from a batch exporter in your application or OpenTelemetry Collector. The sidecar processes spans in the background and stores findings in SQLite. Declare stable source IDs, versions, and rule scopes to connect a finding to the original input. [Deploy and configure capture](docs/sidecar.md)

Check an assembled request before serving it with [preflight](docs/guide.md#check-context-before-inference). Use a [controlled replay](docs/guide.md#try-a-decision-replay) to measure how an edit changes a selected outcome. [Read what each finding supports](docs/methods.md).

## Project

NexusWeave uses the [Apache License 2.0](LICENSE). The retail trace excerpt uses [CDLA Permissive 2.0](LICENSES/CDLA-Permissive-2.0.txt). [Contributing](CONTRIBUTING.md) · [Research and dataset credits](CREDITS.md)
