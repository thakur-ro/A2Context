# What can we tell before an LLM answers?

A workflow engineer can check that the pending request contains a declared rule, see which sources disagree, and use an earlier replay to choose an experiment. Inferring which undeclared piece of knowledge is missing is a harder problem. Our first held-out detector missed 19 of 20 removed passages. We kept it out of the recommendation feed. [Experiment design, scores and reproduction](../examples/public-qa/annotation-gap.md)

## Follow a source through a workflow

![Scoped graph of sources, findings and outcomes in the multi-workflow fixture](media/context-network.png)

Choose **Show the context map**. The overview uses the representative evidence returned for each finding in the current filter scope. A source can connect to several findings. Select it to see those relationships, then select a finding or a value to open the exact call. Declared values come from captured instructions. Trial outcomes come from attached comparisons. Matching a source across findings uses its producer, organization, workflow, identifier, revision and role. Unnamed items stay tied to their call.

The fixture above contains 43 findings. The overview draws up to 120 nodes and the picker includes every loaded node. The layout brings linked nodes closer for navigation. Edge length carries no importance score. To investigate model internals, open the exchange demo’s **Before inference** view, select the measured request, then **Inspect both activation positions**. Each dot there represents one intervention at a known decoder block and token position. [Walkthrough](demo.md#see-what-the-model-did-on-the-pending-request)

## Evidence ledger for 20 November 2026

| Question | Result | Engineering decision |
| --- | --- | --- |
| Can we detect a known rule capture gap? | With an explicit rule key and scope, preflight counts matching declarations and known values before inference. A request can have zero conflicts and zero declarations. | Show the missing capture before inference and direct the engineer to the request builder. [Preflight check](guide.md#check-context-before-inference) |
| Does the first-token distribution identify a useful passage to edit? | Editing the highest-ranked passage changed 14 of 17 scorable answers. Editing the first passage changed 11 of 17. Only five pairs differed on this outcome, with exact paired two-sided p = 0.375. Three pairs were unscorable. | Offer the ranking as an experiment selector. The holdout leaves its advantage over first position uncertain. [Paired holdout](../examples/public-qa/holdout.md) |
| Can the model warn that an undeclared passage is missing? | On 20 new questions, Qwen’s score for the instructed abstention flagged one missing request and the intact version of that question. Negative maximum BM25 also detected one missing request and flagged three intact ones. Both measures scored the missing variant higher in 8 of 20 pairs. | Keep both scores experimental. Two complete CPU runs reproduced the saved 60-prompt result, including the ten pilot pairs. [Omission study](../examples/public-qa/annotation-gap.md) |
| What did a tested internal change affect? | On the held-out exchange request, swapping one vector at the supplied tool-call prefix shifted the score by up to −0.55 log odds for the cache edit and +0.74 for the customer edit. No tested swap reversed the token preference. | Expose the block, position, baseline, patched score and self-swap together. Check generated arguments separately. [Exchange measurements](demo.md#see-what-the-model-did-on-the-pending-request) |

For the passage ranking, the statistic is Jensen–Shannon divergence between the next-token distributions of the original request and the edited request. For a block intervention, the statistic is the change in log odds between two specified tokens at a fixed prefix. Neither quantity identifies a complete computation. An ordinary provider trace supplies no intermediate activations. The research contribution we can test is whether linking these limited measurements back to sources and workflow outcomes helps engineers choose better interventions. Existing attribution and circuit-tracing methods already address parts of that question. [Methods](methods.md) · [Scoping review](research-review.md) · [Activation patching guide](https://arxiv.org/abs/2404.15255v1) · [Circuit tracing methods](https://transformer-circuits.pub/2025/attribution-graphs/methods.html)

## Next experiment

Does a pending request lack the evidence needed for a *specific outcome*? The omission experiment labeled the removal of an annotated passage. An alternative passage might still answer the question. Two of the twenty missing variants even contained a reference answer alias.

1. Reserve fresh questions before choosing thresholds. For every question, label answer support in each passage set without seeing detector scores. Keep separate counts for ambiguous support and alternate supporting passages.
2. Run the specified abstention score and lexical baseline at their original thresholds. Add a support classifier chosen on pilot questions only. Record the extra model calls, tokens and elapsed time per pending request.
3. Compare misses and false alarms on the same held-out pairs, retaining all unscorable cases. Inspect each disagreement against its evidence labels. If changing a threshold or instruction, evaluate it on another untouched set. Keep the detector experimental until there is a reproducible benefit at a false-alarm rate the workflow owner can accept.

## Release scope

A source preview can ship the deterministic checks, trace adapters, UI and reproducible experiments. The sidecar and browser journeys have scripted fixture coverage. A staging pilot with an independently instrumented workflow still needs to establish whether engineers can follow a recommendation unaided and whether it saves an unnecessary replay. Two items remain before changing visibility. Get a completed hosted CI run, and finish checking the terms for the Natural Questions and Wikipedia material in the bundled QA examples. The upstream experiment repository licenses its software under MIT. The full passage corpus downloads separately. [Upstream license](https://github.com/nelson-liu/lost-in-the-middle/blob/29b8a6d042ce29abccee3db1a73171a107d7e6af/LICENSE) Inference accuracy and deployment cost require separate evidence on the target workflow. [Validation and resource measurements](validation.md) · [Dataset and software credits](../CREDITS.md)
