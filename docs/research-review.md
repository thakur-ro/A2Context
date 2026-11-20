# Related work on diagnosing LLM context

**Scope search on 19 November 2026.** The question was which existing systems expose source-level context problems at runtime, find them across traces, or test how a context change affects an outcome. This is a targeted scoping review of work relevant to the implementation. Coverage is limited to sources found and verified under the searches below.

## Search and selection

We searched the web for primary sources in arXiv, NeurIPS, ICLR, ACL Anthology, official product documentation, and GitHub. We started with the closest trace-analysis systems, then looked up the original methods and checked their public interfaces. We used these queries and close variants.

- `TRACE agent trajectory context engineering context failures`
- `REFLECT replay agent trajectory error attribution patch`
- `LangSmith Insights trace patterns failure modes` and `SentinelRCA deterministic detectors traces`
- `context attribution source ablation LLM ContextCite AttriBoT`
- `IHEval conflicting instruction hierarchy` and `activation patching Heimersheim Nanda interpret interventions`
- `context-render Claude Code source context view` and `Langfuse trace sessions metadata`

We included primary sources that describe an implemented or evaluated method for trace mining, context provenance, instruction conflict, controlled replay, or internal interventions. We included adjacent observability tools where an existing feature could replace part of the design. We excluded broad LLM monitoring and prompt-management products without a documented mechanism bearing on the question. Preprints and product documentation support claims about their described methods and interfaces. Use the linked paper identifiers and named product documentation to reproduce this reading set.

## Closest systems

| System | Unit of evidence | Documented overlap | What to reuse or test |
| --- | --- | --- | --- |
| **TRACE** | Historical agent trajectories and context artifacts | Mines trajectories for context failures and proposes edits to context assets. The preprint reports simulated-benchmark evaluation. | Treat context failure mining as established adjacent work. Compare on how cheaply a finding can preserve an exact source and lead to a reproducible test. [TRACE, arXiv:2608.09153](https://arxiv.org/abs/2608.09153v1) |
| **REFLECT** | Candidate error steps and controlled trajectory replays | Uses patch and replay to refine attribution of silent failures after execution. | Controlled replay is shared ground. Evaluate whether a targeted source edit and named outcome are enough to make the next engineering step reproducible. [REFLECT, arXiv:2606.09071](https://arxiv.org/abs/2606.09071v1) |
| **LangSmith Insights** | Patterns and failure modes across agent traces | Groups behaviors from trace data, with thread-level evaluation and drill-down. | Keep trace, span, thread, and metadata terminology compatible. Test source-linked findings and replay as a companion workflow. [Insights](https://www.langchain.com/blog/insights-agent-multiturn-evals-langsmith) · [Entity model](https://docs.langchain.com/langsmith/observability-concepts) |
| **SentinelRCA** | Structural trace and telemetry signals | Applies deterministic detectors for issues such as retries using existing telemetry. | Benchmark a small deterministic detector separately from export cost. Test the added data needed for context-level evidence. [SentinelRCA](https://github.com/sentinelrca/sentinel/tree/a4d2d363759d55a81b968a5bcb5eefb3ceb43960) |
| **context-render** | Claude Code context sources and sessions | Makes named sources and their use across coding sessions visible. | Keep stable source identities and compare sessions. Test the normalization contract with producers outside coding agents. [context-render](https://github.com/gigayaya/context-render/tree/a08d72cd57801778571b18b82e5ab421d73f7b0a) |
| **Langfuse** | Traces, observations, sessions, metadata | Provides the surrounding observability model and filtering that a sidecar can use alongside its own findings. | Reuse the OTLP pipeline and avoid building another general trace platform. [Data model](https://langfuse.com/docs/observability/data-model) |

This matrix records capabilities described in the reviewed sources. The combination to test is a source-level queue of findings with exact evidence locations, bounded default processing, and a controlled follow-up. There is no measured head-to-head result against these systems in this review.

## Attribution and experimental methods

| Source | Method in the primary source | Relationship to NexusWeave |
| --- | --- | --- |
| **ContextCite** | Perturbs context to attribute generated statements to source passages. | Related. A NexusWeave replay tests a selected edit and selected answer or tool outcome. It does not fit ContextCite’s attribution model. [NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/hash/adbea136219b64db96a9941e4249a857-Abstract-Conference.html) |
| **AttriBoT** | Approximates leave-one-out influence on a generated response with efficient scoring. | Related. A candidate for a separate evaluator with an explicit cost budget. [ICLR 2025](https://proceedings.iclr.cc/paper_files/paper/2025/hash/2aab664e0d1656e8b56c74f868e1ea69-Abstract-Conference.html) |
| **IHEval** | Evaluates instruction following when requests at different levels agree or conflict. | Related benchmark for model behavior. Its labels do not establish source-level causal effects for production traces. [NAACL 2025](https://aclanthology.org/2025.naacl-long.425/) |
| **Activation patching guide** | Explains activation interventions, metrics, and interpretation. | Adapted in optional instrumented examples. A patch measures an outcome under that intervention for the tested model and inputs. [arXiv:2404.15255](https://arxiv.org/abs/2404.15255) |

Captured trace inputs show what the application reports supplying. Measuring how an edit changes behavior requires an intervention, matched trial conditions, and a defined outcome. The [methods guide](methods.md) describes the evidence contract for each finding and lists what can be reproduced.
