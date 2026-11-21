# Credits

A2Context draws on prior work in source attribution, instruction hierarchy and controlled replay. The literature review links the original publications and comparison systems. [Research and related systems](docs/research-review.md)

The exchange scenario was inspired by Exgentic's retail workflow. The distributed walkthrough uses constructed order records and a deterministic proposal fixture. [Exgentic dataset](https://huggingface.co/datasets/Exgentic/agent-llm-traces) · [Data license](LICENSES/CDLA-Permissive-2.0.txt)

## Methods

Giga Chang’s context-render informed the use of named sources across sessions. Victor Klee’s interval sweep underlies the check for overlapping source ranges. [context-render](https://github.com/gigayaya/context-render) · [Klee, 1977](https://doi.org/10.1080/00029890.1977.11994336)

Stefan Heimersheim and Neel Nanda’s guide informed the decoder-block intervention controls. The replay intervals use Wassily Hoeffding’s bound under independent sampling. [Activation patching guide](https://arxiv.org/abs/2404.15255) · [Hoeffding, 1963](https://doi.org/10.1080/01621459.1963.10500830)

## Interoperability

The local stdio adapter uses the official Rust SDK for Model Context Protocol. [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

The embedded conflict view uses the official MCP Apps browser SDK. Its dependency licenses are included with the bundled view. [MCP Apps SDK](https://github.com/modelcontextprotocol/ext-apps), [Bundled JavaScript licenses](LICENSES/MCP-App-JavaScript.txt)

The dashboard uses the official Lit renderer and Basic Catalog for A2UI recommendation cards. [A2UI project](https://github.com/a2ui-project/a2ui) · [Bundled JavaScript licenses](LICENSES/A2UI-JavaScript.txt)

The importers follow trace and attribute formats published by OpenTelemetry, OpenInference, and LangSmith. [OTLP specification](https://opentelemetry.io/docs/specs/otlp/), [OpenInference semantic conventions](https://arize-ai.github.io/openinference/spec/attributes/semantic-conventions.html), [LangSmith run format](https://docs.langchain.com/langsmith/run-data-format)
