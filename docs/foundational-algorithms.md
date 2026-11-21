# How A2Context finds context problems

A2Context uses the request and response evidence supplied by the application. A tool definition establishes what the agent was offered. A tool proposal establishes what it requested. A subsequent tool message can establish what reached a later model call when its invocation ID matches. Missing capture leaves the corresponding question open.

## Recommendations

- Compare declared rules that share a key and scope. A disagreement identifies two input sources to inspect.
- Check proposed arguments against the tool schema in the same request. A missing required field justifies inspecting dispatch.
- Compare identifiers in a reply's references with shared lines in consecutive eligible requests. The card points to the same text in two conversations. The application can verify whether the reference belongs to the retrieved document.
- Group related observations by their recorded identities and comparable conditions. Follow a group to a specific call and source location.
- Attach original and edited trials to measure an outcome under a named change. An edit can influence the emitted text even when both answers are incorrect.

## A score for the monitored workflow

**Proposed measure.** Report the percentage of assessable runs that passed a named set of operational checks. Start with one capability, such as tool handoffs. An example check asks whether a tool result designated for delivery appears in the next model request with its invocation ID. Apply that check only to runs with a handoff due. Intentional termination, an expected human wait and legitimate result transformations need explicit treatment in the check definition. Publish the check set and version. Teams can use the built-in checks without supplying test cases or grading answers.

For one workflow version, environment and time window, count distinct eligible runs. Use the underlying evidence for recommendations before ranking, suppression or display. Each run receives exactly one status.

| Count | Meaning |
| --- | --- |
| $B$ | Evidence establishes at least one monitored incident. Three recommendations about the same run still count once. |
| $C$ | At least one check applied, and every applicable check completed with a pass. |
| $U$ | The available evidence leaves the status unresolved. For example, the next request was only partially captured. |

With $N=B+C+U$, the **monitored pass rate** is $100C/(B+C)$ and **assessment coverage** is $100(B+C)/N$. Report both with the counts. When $B+C=0$, show “insufficient evidence.” Use an independent application run count when available to quantify runs missing from telemetry altogether. Otherwise limit the claim to recorded eligible runs. The event-ratio construction follows established service-level indicator practice. [Google SRE Workbook](https://sre.google/workbook/implementing-slos/)

For illustration, 882 passes, 18 affected runs and 100 unresolved runs give a 98% pass rate at 90% coverage. If every unresolved run failed, 88.2% of the eligible cohort passed. If every unresolved run passed, 98.2% passed. These bounds, $[100C/N,\;100(C+U)/N]$, reflect missing evidence even if missingness is related to failure. They assume resolved classifications are correct. They describe this cohort and supply no statistical guarantee about future traffic. [Horowitz and Manski on incomplete data](https://www.sciencedirect.com/science/article/pii/S030440760500062X)

**Available check.** Ask **Show the tool reference score** in the interview or query `GET /api/sidecar/tool-reference-score`. Version 1 checks a narrower property than delivery: each tool-role result in a recorded chat request must have a nonempty ID matching exactly one preceding assistant tool call in that request. It measures distinct normalized traces in the selected scope and trace-start window. The interview gives counts, missing-data bounds, and up to five affected traces with the exact message position. It can reveal a broken reference in an assembled history. Establishing that a result never arrived, that its payload was correct, or that a task succeeded requires other evidence. A trace with known tool-result input but incomplete evidence can be unresolved. The API reports traces of unknown applicability and fully inspected traces outside the check separately from the eligible denominator.
