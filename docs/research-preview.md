# What can we tell before an LLM answers?

A workflow engineer can check that the pending request contains a declared rule, see which sources disagree, and use an earlier replay to choose an experiment. Inferring which undeclared piece of knowledge is missing is a harder problem. Our first held-out detector missed 19 of 20 removed passages. We kept it out of the recommendation feed. [Experiment design, scores and reproduction](../examples/public-qa/annotation-gap.md)

## Follow a source through a workflow

![Scoped graph of sources, findings and outcomes in the multi-workflow fixture](media/context-network.png)

Choose **Show the context map**. The overview uses the representative evidence returned for each finding in the current filter scope. A source can connect to several findings. Select it to see those relationships, then select a finding or a value to open the exact call. Declared values come from captured instructions. Trial outcomes come from attached comparisons. Matching a source across findings uses its producer, organization, workflow, identifier, revision and role. Unnamed items stay tied to their call.

Select a first-token finding to compare the source edits before generation. For a candidate-score finding, the two branches show log odds at a fixed prefix. A token finding shows the original token’s probability before and after the named edit. In the example below, editing the refund policy reduced the probability assigned to the token `4` at output position 14 from 99.97% to about 0.0011%. The bar chart shows nearby token positions. The decoder-block swap is a separate measurement on the same output path.

![Context map showing the original and edited probabilities for the token 4 and the measured decoder-block swap](media/context-token.png)

Choose **Follow source edits across this answer** to see which edits mattered at different points in the same generation. In the recorded sentence, **returns-policy** has the largest measured effect on the return-window number at position 6. **refund-policy** has the largest effect on the refund time at position 14. Selecting a cell shows the exact edit, the score change at that prefix and both freely generated answers. Untested inputs appear below the measurements. From the recommendation detail page, the address includes the selected token and source edit so another engineer can open the same comparison.

![Three source edits across the sixteen tokens of a returns answer, with the refund time selected](media/source-edits-by-token.png)

Each cell compares two log probabilities for the same output token with all earlier output tokens fixed. The sign shows direction and the color intensity uses one scale for the entire answer. The report requires access to a model that can score edited requests. A workflow trace alone supplies no cell values. [Measurement and generation conditions](methods.md) · [Saved CPU measurements](../examples/runtime-comparison/output-token-report-transformers.json)

The overview fixture contains 43 findings. The overview draws up to 120 nodes and the picker includes every loaded node. The layout brings linked nodes closer for navigation. Edge length carries no importance score. To investigate model internals, open the exchange demo’s **Before inference** view, select the measured request, then **Inspect both activation positions**. Each dot there represents one intervention at a known decoder block and token position. [Walkthrough](demo.md#see-what-the-model-did-on-the-pending-request)

## Evidence ledger for 20 November 2026

| Question | Result | Engineering decision |
| --- | --- | --- |
| Can we detect a known rule capture gap? | With an explicit rule key and scope, preflight counts matching declarations and known values before inference. A request can have zero conflicts and zero declarations. | Show the missing capture before inference and direct the engineer to the request builder. [Preflight check](guide.md#check-context-before-inference) |
| Does the first-token distribution identify a useful passage to edit? | Editing the highest-ranked passage changed 14 of 17 scorable answers. Editing the first passage changed 11 of 17. Only five pairs differed on this outcome, with exact paired two-sided p = 0.375. Three pairs were unscorable. | Offer the ranking as an experiment selector. The holdout leaves its advantage over first position uncertain. [Paired holdout](../examples/public-qa/holdout.md) |
| Can the model warn that an undeclared passage is missing? | On 20 new questions, Qwen’s score for the instructed abstention flagged one missing request and the intact version of that question. Negative maximum BM25 also detected one missing request and flagged three intact ones. Both measures scored the missing variant higher in 8 of 20 pairs. | Keep both scores experimental. Two complete CPU runs reproduced the saved 60-prompt result, including the ten pilot pairs. [Omission study](../examples/public-qa/annotation-gap.md) |
| What did a tested internal change affect? | On the held-out exchange request, swapping one vector at the supplied tool-call prefix shifted the score by up to −0.55 log odds for the cache edit and +0.74 for the customer edit. No tested swap reversed the token preference. | Expose the block, position, baseline, patched score and self-swap together. Check generated arguments separately. [Exchange measurements](demo.md#see-what-the-model-did-on-the-pending-request) |

For the passage ranking, the statistic is Jensen–Shannon divergence between the next-token distributions of the original request and the edited request. For a block intervention, the statistic is the change in log odds between two specified tokens at a fixed prefix. Neither quantity identifies a complete computation. An ordinary provider trace supplies no intermediate activations. The research contribution we can test is whether linking these limited measurements back to sources and workflow outcomes helps engineers choose better interventions. Existing attribution and circuit-tracing methods already address parts of that question. [Methods](methods.md) · [Scoping review](research-review.md) · [Activation patching guide](https://arxiv.org/abs/2404.15255v1) · [Circuit tracing methods](https://transformer-circuits.pub/2025/attribution-graphs/methods.html)

## What the next two studies changed

Removing the annotated passage still left sufficient evidence for a plausible answer in eight of twelve pairs, according to a score-blind AI review. A pre-answer support question caught one of the three requests that lacked an identified answer and flagged six of twenty supported requests. One uncertain request was excluded. On 48 human-labeled, single-paragraph SQuAD questions, the same threshold caught 15 of 24 unsupported questions and flagged one of 24 supported questions. These samples do not establish a reliable general alert for a production retriever. [Paired support study](../examples/public-qa/support-review/README.md) · [Independent benchmark](../examples/public-qa/support-review/transfer/README.md)

## From a support guess to a measured edit

We used the reviewed evidence sets to choose the smallest group of passages whose removal would break every identified route to an answer. Each question also got a control edit touching the same number of unrelated passages, chosen for similar length and position. Qwen’s greedy answer changed after the evidence edit on ten of twelve questions and after the control edit on six. Five answers changed only after the evidence edit. One changed only after the control edit. Five changed under both edits and one stayed the same throughout. All 36 generations completed. [Frozen selection and matching procedure](../examples/public-qa/support-review/witness-edits/protocol.md) · [Outputs and reproduction](../examples/public-qa/support-review/witness-edits/README.md)

One evidence edit only changed quotation marks. Only two original answers contained a reference alias. An automatic recommendation based on a text difference would give punctuation the same weight as a change of answer. The current actionable results are the *specific observed edits*, with their original outputs and controls available for review.

## Does a pre-answer source ranking find a change in business state?

In eight new synthetic exchange requests, the pre-answer divergence ranking selected the latest customer wording every time. Before enforcing approval, the selected edit changed the pending order on four requests. The first listed edit did so on one. Two wording edits restored the requested hold and the reverse edits lost it. Choosing the latest customer message directly would have selected the same eight edits at zero scoring cost. With the example adapter and an approval check at dispatch, all 32 edits preserved the final order state. These eight requests are paired wordings of one workflow. [Registered design, local measurements and reproduction](../examples/exchange-approval/dispatch-study/README.md)

Changing only the tool parser changed which four requests the top edit affected. The example adapter accepted 30 of 40 generations as executable tool calls. Requiring the inner payload to parse as JSON accepted eight. Applying the same approval gate still left twelve final states that differed between parsers. A production replay therefore needs the actual parser and dispatch policy as well as the model and prompt. [Per-arm evidence](../examples/exchange-approval/dispatch-study/results.json) · [Offline parser replay](../examples/exchange-approval/dispatch-study/compare_parsers.py)

For a known staging requirement, preflight now checks whether at least one complete set of named source fragments is anchored in the request. An exchange can be configured to require the cache and latest customer message together, or an independent consent event. With a matching version-two receipt, the sidecar also groups repeated gaps across conversations and links back to the calls. A successful check establishes that the configured IDs are present. The live authorization decision still belongs at dispatch. [Check alternative sources](guide.md#check-alternative-sources-before-dispatch)

## What did an independent agent runtime actually execute?

Qwen-Agent’s native tool parser and registered tool callback replayed the same 40 previously generated responses. It recognized 30 tool calls, rejected 22 argument payloads and executed eight. Relative to the original adapter, the pending state differed on 12 of 40 trials even with the approval gate enabled. Source sensitivity changed with the execution stack. The ranked edit affected four approved requests under Qwen-Agent and four held requests under the original ungated adapter.

Of those four approved exchanges, the ranked edit restored two expected writes and lost two. Counting any changed outcome would have reported all four as successes. For a recommendation to say that an edit helped, the workflow needs to supply its expected outcome.

The harness exported 236 application-instrumented OpenTelemetry spans to a disposable sidecar across 80 traces and eight threads. The two dispatch policies reused the same generated text. The exporter captured stage boundaries without prompt or response bodies. These spans establish where a proposal stopped. They cannot establish which context the model used. [Framework implementation, saved receipts and sidecar check](../examples/exchange-approval/dispatch-study/framework/README.md)

## Next validation

We allowed one fresh local generation after each of the 22 argument rejections. Every retry produced plain text. Twelve approved exchange trials still lacked their expected write. The pre-answer ranking picked the latest customer message on all eight requests. A selector based only on message position would have tested the same edits on these requests. The result supports keeping model-based ranking opt-in. [Per-request figure, retry protocol and reproduction](../examples/exchange-approval/dispatch-study/framework/README.md#what-happened-when-the-model-could-try-again)

The next test needs a second workflow where evidence can come from a retrieved document, a tool result or the latest message. Register its expected outcome and compare the ranking to latest-message and first-listed selection before proposing a default. Use preflight to check whether the edits share request bytes. Have a workflow engineer investigate an affected request without guidance and record time to identify the failed stage and whether the recommended edit restores the outcome on a fresh request. The current fixtures supply no independent usability result.

We froze five trial-zero checkpoints from Sierra’s public airline submission. Each follows a linked reservation lookup and stops immediately before an agent response. Across those checkpoints, all 19 controlled substitutions kept their source ID and passed the name-only preflight check. With independently pinned text digests, preflight flagged all 19 substitutions, distinguished all 19 missing-anchor controls, and passed all five intact requests. The source ranges were disjoint in each request. This measures whether exact upstream text survived assembly. It requires the workflow to know which text to expect. [Frozen checkpoints](../examples/airline-transfer/selection.json) · [Capture results](../examples/airline-transfer/capture-results.json)

To reproduce the capture comparison after downloading the pinned inputs, build the CLI and run:

```sh
cargo build --locked
python3 examples/airline-transfer/check_capture.py --data /tmp/airline-study --binary target/debug/nexusweave --check
```


## Release scope

A source preview can ship the deterministic checks, trace adapters, UI and reproducible experiments. The sidecar and browser journeys have scripted fixture coverage. The release still needs a completed hosted CI run. GitHub declined to start the runners for the last validation push. The public QA examples now include upstream data notices and an article index for the 62 questions used in the passage studies. The SQuAD transfer example ships identifiers, labels, hashes and measurements, with the full text downloaded separately for reproduction. [QA data credits](../examples/public-qa/data-credits.md) · [SQuAD source and license](https://rajpurkar.github.io/SQuAD-explorer/)

Inference accuracy and deployment cost require separate evidence on the target workflow. [Validation and resource measurements](validation.md) · [Dataset and software credits](../CREDITS.md)
