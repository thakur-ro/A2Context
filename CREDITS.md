# Credits

## Context inspection and replay

- Giga Chang’s `context-render` informed the use of named context sources and cross-session views. It is an independent project for Claude Code. [context-render](https://github.com/gigayaya/context-render)
- Benjamin Cohen-Wang, Harshay Shah, Kristian Georgiev, and Aleksander Mądry introduced ContextCite, a related method for attributing generated statements to context. NexusWeave’s replay tests user-selected edits against recorded workflow decisions. [ContextCite, NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/hash/adbea136219b64db96a9941e4249a857-Abstract-Conference.html)
- Stefan Heimersheim and Neel Nanda describe how to apply and interpret activation patching. Their guide informed the decoder-block swaps and controls in the instrumented examples. [How to use and interpret activation patching](https://arxiv.org/abs/2404.15255)
- Mor Geva, Jasmijn Bastings, Katja Filippova, and Amir Globerson use interventions on attention edges to study information flow. Their work informs the source visibility probe. AttriBoT is a related implementation for efficient leave-one-out context attribution. [Geva et al., EMNLP 2023](https://aclanthology.org/2023.emnlp-main.751/), [AttriBoT, ICLR 2025](https://proceedings.iclr.cc/paper_files/paper/2025/hash/2aab664e0d1656e8b56c74f868e1ea69-Abstract-Conference.html)
- Marc Lipsitch, Eric Tchetgen Tchetgen, and Ted Cohen describe negative controls for probing bias. NexusWeave adapts that experimental idea by comparing a target edit with an edit expected to leave the decision unchanged. [Negative Controls, 2010](https://pmc.ncbi.nlm.nih.gov/articles/PMC3053408/)
- Aaron Mueller’s analysis of independently sufficient causes motivated the synthetic two-source dispatch experiment. Peter Hase and colleagues tested limits on using localization to select model edits. [Mueller, 2024](https://arxiv.org/abs/2407.04690v1) · [Hase et al., NeurIPS 2023](https://arxiv.org/abs/2301.04213v2)
- Wassily Hoeffding’s inequality supplies the bound used for reported replay intervals under independent sampling. The paired-edit test uses the four conditions of a two-factor design. [Hoeffding, 1963](https://doi.org/10.1080/01621459.1963.10500830), [NIST factorial experiment definition](https://www.itl.nist.gov/div898/handbook/glossary.htm)

The [scoping review](docs/research-review.md) compares the sidecar workflow with TRACE, REFLECT, LangSmith Insights, SentinelRCA, and Langfuse. The README structure uses [Transformers](https://github.com/huggingface/transformers) as a reference for showing a runnable result near the top.

## Data and tools

- Exgentic’s `agent-llm-traces` dataset supplied the recorded retail sessions for the importer comparison. The [sample manifest](examples/retail-sample.manifest.json) pins the dataset revision and documents session selection. The policy excerpt in [retail-annotations.json](examples/retail-annotations.json) comes from that dataset and is distributed under [CDLA Permissive 2.0](LICENSES/CDLA-Permissive-2.0.txt). [Exgentic dataset card](https://huggingface.co/datasets/Exgentic/agent-llm-traces)
- The retail sessions come from the τ²-Bench benchmark, by Victor Barres, Honghua Dong, Soham Ray, Xujie Si, and Karthik Narasimhan. [τ²-Bench paper](https://proceedings.mlr.press/v306/barres26a.html)
- The retail reference comparison uses `agentpile` as a trace viewer. [agentpile](https://github.com/yzhao062/agentpile)
- The saved local replay examples use Liquid AI’s LFM2.5-1.2B-Instruct model through MLX LM. The repository ships synthetic requests and recorded trial outcomes for these examples. [Liquid AI model card](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct), [MLX LM](https://github.com/ml-explore/mlx-lm)
- The cross-runtime experiment also uses the Qwen team's Qwen2.5-0.5B-Instruct checkpoint through Transformers and PyTorch. The repository includes the measured reports and synthetic inputs. [Pinned Qwen model card](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/tree/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2), [Transformers](https://github.com/huggingface/transformers), [PyTorch](https://github.com/pytorch/pytorch)
- The optional context scorer uses Sentence Transformers' Apache-2.0 MiniLM cross-encoder, trained on SNLI and MultiNLI. IHEval provides related evaluation work on instruction hierarchy under conflict. [Pinned MiniLM model card](https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/tree/b95119ce93d3e065de6214e38cd4a97b0f2f2c6d) · [SNLI](https://aclanthology.org/D15-1075/) · [MultiNLI](https://aclanthology.org/N18-1101/) · [IHEval](https://aclanthology.org/2025.naacl-long.425/)

The public passage-edit experiment uses the multi-document QA data released by Liu et al. and adapts the distribution-based source ranking studied by Li et al. The checkpoint, dataset and method retain their respective attribution. [Lost in the Middle data](https://github.com/nelson-liu/lost-in-the-middle/tree/29b8a6d042ce29abccee3db1a73171a107d7e6af) · [ARC-JSD](https://arxiv.org/abs/2505.16415)

The review also compares this work with PECoRe by Gabriele Sarti and colleagues and MIRAGE by Jirui Qi and colleagues. Both use changes in token predictions to study context reliance. [PECoRe](https://gsarti.com/publication/pecore/) · [MIRAGE](https://aclanthology.org/2024.emnlp-main.347/)

## Interoperability

The local stdio adapter uses the official Rust SDK for Model Context Protocol. [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

The embedded conflict view uses the official MCP Apps browser SDK. Its dependency licenses are included with the bundled view. [MCP Apps SDK](https://github.com/modelcontextprotocol/ext-apps), [Bundled JavaScript licenses](LICENSES/MCP-App-JavaScript.txt)

The importers follow trace and attribute formats published by OpenTelemetry, OpenInference, and LangSmith. [OTLP specification](https://opentelemetry.io/docs/specs/otlp/), [OpenInference semantic conventions](https://arize-ai.github.io/openinference/spec/attributes/semantic-conventions.html), [LangSmith run format](https://docs.langchain.com/langsmith/run-data-format)
