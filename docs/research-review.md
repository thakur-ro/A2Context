# Related work on diagnosing LLM context

## Search and selection

We searched the web for primary sources in arXiv, NeurIPS, ICLR, ACL Anthology, official product documentation, and GitHub. We started with the closest trace-analysis systems, then looked up the original methods and checked their public interfaces. We used these queries and close variants.

- `TRACE agent trajectory context engineering context failures`
- `REFLECT replay agent trajectory error attribution patch`
- `LangSmith Insights trace patterns failure modes` and `SentinelRCA deterministic detectors traces`
- `context attribution source ablation LLM ContextCite AttriBoT`
- `language model context attribution Jensen Shannon divergence passage ablation ARC-JSD` and `Lost in the Middle Natural Questions gold at 4`
- Follow-up on 20 November 2026: `context-sensitive token identification PECoRe` and `model internals RAG context sensitivity MIRAGE`.
- `IHEval conflicting instruction hierarchy` and `activation patching Heimersheim Nanda interpret interventions`
- `language model sensitivity prompt format Sclar performance spread`
- `counterfactual interventions multiple independently sufficient causes language models` and `does causal localization inform model editing`
- Follow-up on 20 November 2026 used `site:aclanthology.org Longpre Entity-Based Knowledge Conflicts in Question Answering substitution contextual answer 2021`.
- The 20 November 2026 reference-answer follow-up used `RAGChecker fine-grained framework diagnosing retrieval augmented generation context utilization claim checker Amazon NeurIPS official github`.
- `context-render Claude Code source context view` and `Langfuse trace sessions metadata`
- The 20 November checkpoint follow-up searched for `counterfactual agent trajectory replay intervening on tool observations assistant messages LLM causal tool selection history benchmark paper`. We followed the CAR paper to its implementation and checked the context-edit grammar, forward runner and attribution entry points at the cited revision.
- The 20 November sufficiency search covered pre-answer RAG context judges and `multi hop question answering remove supporting paragraphs contrastive support DiRe`.
- The 20 November transfer follow-up searched for `AppWorld benchmark state based evaluation` and `MAST multi agent failure taxonomy trace annotations`. It followed the MAST dataset to the AdaMAST runtime in ATLAS.
- The step-attribution follow-up searched for `Who When failure attribution benchmark` and `Who&When Pro failure localisation injection`. It followed both datasets to their authors' releases and inspected the released step coordinate mappings for smolagents and ALFWorld.

We included primary sources that describe an implemented or evaluated method for trace mining, context provenance, instruction conflict, controlled replay, or internal interventions. We included adjacent observability tools where an existing feature could replace part of the design. We excluded broad LLM monitoring and prompt-management products without a documented mechanism bearing on the question. Preprints and product documentation support claims about their described methods and interfaces. Use the linked paper identifiers and named product documentation to reproduce this reading set.

## Systems and methods

| Work | What it examines | Primary sources |
| --- | --- | --- |
| **TRACE** | Historical agent trajectories and context artifacts | [TRACE, arXiv:2608.09153](https://arxiv.org/abs/2608.09153v1) |
| **REFLECT** | Candidate error steps and controlled trajectory replays | [REFLECT, arXiv:2606.09071](https://arxiv.org/abs/2606.09071v1) |
| **LangSmith Insights** | Patterns and failure modes across agent traces | [Insights](https://www.langchain.com/blog/insights-agent-multiturn-evals-langsmith) · [Entity model](https://docs.langchain.com/langsmith/observability-concepts) |
| **SentinelRCA** | Structural trace and telemetry signals | [SentinelRCA](https://github.com/sentinelrca/sentinel/tree/a4d2d363759d55a81b968a5bcb5eefb3ceb43960) |
| **Langfuse** | Traces, observations, sessions, metadata | [Data model](https://langfuse.com/docs/observability/data-model) |
| **ContextCite** | Perturbs context to attribute generated statements to source passages. | [NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/hash/adbea136219b64db96a9941e4249a857-Abstract-Conference.html) |
| **AttriBoT** | Approximates leave-one-out influence on a generated response with efficient scoring. | [ICLR 2025](https://proceedings.iclr.cc/paper_files/paper/2025/hash/2aab664e0d1656e8b56c74f868e1ea69-Abstract-Conference.html) |
| **IHEval** | Evaluates instruction following when requests at different levels agree or conflict. | [NAACL 2025](https://aclanthology.org/2025.naacl-long.425/) |
| **Causal Agent Replay (CAR)** | Intervention on an agent step, followed by new trajectories and a user-supplied outcome | [Shah, preprint](https://arxiv.org/abs/2606.08275v1) · [Context-edit implementation](https://github.com/jaineet17/causal-agent-replay/blob/db90fa28b97164c35e7c524e4597b8cbdb3035af/src/car/replay/intervene.py) · [Attribution package](https://github.com/jaineet17/causal-agent-replay/tree/db90fa28b97164c35e7c524e4597b8cbdb3035af/src/car/attribute) |
| **MemUse** | Delivered memories, prior visible context, subsequent actions | [Xu, Li and Xu, preprint](https://arxiv.org/abs/2611.01718v1) |
| **context-render** | Claude Code context sources and sessions | [context-render](https://github.com/gigayaya/context-render/tree/a08d72cd57801778571b18b82e5ab421d73f7b0a) |
| **RAGChecker** | Claims in answers, reference answers and retrieved chunks | [Ru et al., NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/hash/27245589131d17368cccdfa990cbf16e-Abstract.html) · [RAGChecker v0.1.0](https://github.com/amazon-science/RAGChecker/releases/tag/v0.1.0) |
| **CAREFUL-Bench** | Executed calls, intermediate effects and final service state in BFCL simulations | [Kovalenko and Redchych, preprint v2](https://arxiv.org/abs/2611.02978v2) |
| **Contrast sets** | Gardner et al. asked dataset authors to make small changes to examples that often change the correct label, then measured performance on the edited examples. | [Gardner et al., Findings of EMNLP 2020](https://aclanthology.org/2020.findings-emnlp.117/) |
| **Arize Phoenix dataset evaluators** | An experiment can score outputs against dataset references with a built-in code check or an LLM evaluator. The Playground runs attached evaluators automatically. Programmatic experiments accept evaluators explicitly. | [Phoenix evaluator documentation, pinned](https://github.com/Arize-ai/phoenix/blob/0ae95cabfc4b1a0cde13371627fe692895588a3a/docs/phoenix/datasets-and-experiments/how-to-experiments/how-to-dataset-evaluators.mdx) |
| **PECoRe and MIRAGE** | PECoRe identifies context-sensitive output tokens and relates them to contextual cues. MIRAGE applies this approach to attributing generated RAG answers. | [Sarti et al., ICLR 2024](https://proceedings.iclr.cc/paper_files/paper/2024/hash/6d3040941a2d57ead4043556a70dd728-Abstract-Conference.html) · [Qi et al., EMNLP 2024](https://aclanthology.org/2024.emnlp-main.347/) |
| **ARC-JSD** | Ranks context spans by changes in model token distributions over a generated response after ablating each span. The paper also examines internal components. | [Li et al., ICLR 2026](https://arxiv.org/abs/2505.16415v5) · [Pinned implementation](https://github.com/ruizheliUOA/ARC_JSD/tree/6572d3be974c6eca69d9754d360756a5e6397dae) |
| **Activation patching guide** | Explains activation interventions, metrics, and interpretation. | [arXiv:2404.15255](https://arxiv.org/abs/2404.15255) |
| **Unfaithful explanations** | Controlled prompt perturbations changed predictions while explanations omitted the triggering input. | [Turpin et al., NeurIPS 2023](https://proceedings.neurips.cc/paper_files/paper/2023/hash/ed3fea9033a80fea1376299fa7863f4a-Abstract-Conference.html) |
| **Circuit tracing** | Attribution graphs and follow-up activation interventions probe selected mechanisms of a particular instrumented model. | [Lindsey et al., 2025](https://transformer-circuits.pub/2025/attribution-graphs/biology.html) |
| **Prompt format sensitivity** | Sclar and colleagues varied few-shot prompt formatting and measured changes in task accuracy across plausible formats. | [ICLR 2024](https://proceedings.iclr.cc/paper_files/paper/2024/hash/6c0e99d736da621403018ca7b32b1a4d-Abstract-Conference.html) |
| **Missed causes** | Mueller explains how independently sufficient causes can mask one another under individual interventions. | [Mueller, 2024](https://arxiv.org/abs/2407.04690v1) |
| **Localization and editing** | In factual recall experiments, the layer localized by denoising did not predict the best layer for the tested fact rewrite. | [Hase et al., NeurIPS 2023](https://arxiv.org/abs/2301.04213v2) |
