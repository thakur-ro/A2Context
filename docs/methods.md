# Methods and evidence

| Method | Status | What the result establishes |
| --- | --- | --- |
| **Declared-rule conflict** | Implemented in the sidecar and in [preflight](guide.md#check-context-before-inference). | Two captured inputs in one call declare different structured values for the same rule key and scope. The card includes the captured source IDs and versions. Model behavior and business impact require separate evidence. |
| **Exact repetition** | Implemented in the sidecar. | Nonblank, included text matches byte for byte within a call. The text is grouped before hashing. Intentional repetitions can produce findings. |
| **Verified replay** | Implemented in the sidecar using the existing replay validator. The paired-edit design and negative controls are adapted from [experimental methods](../CREDITS.md#context-inspection-and-replay). | An edit changes the rate at which a defined answer or action matches its scoring rule under the recorded conditions. The validator binds the result to the recorded call, captured baseline request and edited source. It recomputes the summaries from the supplied trials. Runner identity and trial provenance are supplied by the experiment producer. |
| **Local semantic screening** | Implemented as a separate, opt-in offline workflow. | A specified model scores candidate text pairs for review. Treat these as hypotheses and use a controlled edit to measure an outcome. [Context screening](guide.md#screen-captured-text-for-overlap) |
| **Proposal-only shadow probe** | Implemented as a separate, opt-in workflow. | Observes answer or tool proposals under a chosen edit. Use a runner with no execution callback for production tools. [Shadow probe](architecture.md#test-which-source-changes-an-outcome) |
| **Instrumented attribution** | Adapted in [activation experiments](inference-attribution-plan.md). | Token position or target-score measurements apply to the tested intervention, layer, model, and inputs. Captured reports can open from the source trace. [Activation patching guide](https://arxiv.org/abs/2404.15255) |
| **ContextCite, AttriBoT, and IHEval** | Related prior work. | See the [scoping review](research-review.md#attribution-and-experimental-methods) for the comparisons. |

## Grouping and ranking

Within a trace, use the captured call ID to locate each observation. Conflict groups use the rule key, scope, and the source-version pair. Exact-repeat groups add the hash of the matching text and the participating sources. Unnamed sources are trace-local. Replay groups use the existing compatibility key, including the outcome, scoring rule, interventions, runner, model, and sampling design. Groups are counts of captured observations with similar evidence. Their effects are never pooled by the feed.

Findings show recurring conflicts first, followed by replays with a measured difference, individual conflicts, exact repeats, and replays with equal measured rates. A replay group that includes a measured change opens the most recent observation with a change. The card and recurrence counts keep the example separate from the whole group. Within each tier, identified thread count, distinct trace count, and latest observed time are sorted descending. A stable group ID breaks ties. Time filters use the observation timestamp for Findings and the trace start for Traces and Threads. Undated evidence is present in the unfiltered feed and excluded from date-windowed counts.

A numeric zero in a replay means the recorded match rates were equal. A missing result means that the validator could not derive a comparable difference from the evidence. The latter remains visible in the trial details of an attached report and never creates a measured replay card. Counts of captured inputs, declared rules, and replayable requests describe capture coverage. They cannot establish the prevalence of a finding in missing telemetry.

## Reproduction

- Run the saved demo and follow the three [README journeys](../README.md#try-the-demo).
- Use the [validation protocol and results](validation.md) to reproduce the ranking, exporter, and load checks.
- Use the [prior attribution experiments](inference-attribution-plan.md) for model and intervention details. See the [research review](research-review.md) for the assumptions and comparison scope.
