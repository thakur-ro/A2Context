# Architecture

## Sidecar architecture

```mermaid
---
config:
  flowchart:
    nodeSpacing: 22
    rankSpacing: 28
    padding: 10
---
flowchart TB
    accTitle: A2Context sidecar and inference paths
    accDescr: The application owns inference and tool execution. It exports captured traces asynchronously and can request a synchronous preview. The sidecar checks captured context and verifies reports from a separate experiment runner. SQLite backs the recommendations dashboard, where an engineer can prepare another test.

    App["LLM application<br/>Runs inference and tools"]
    Runner["Experiment runner<br/>Tests a source edit"]
    Worker["A2Context sidecar<br/>Context and report checks"]
    Store[("SQLite<br/>Evidence and findings")]
    Dashboard["Recommendations<br/>Dashboard and API"]

    App -->|"Async OTLP/HTTP"| Worker
    App <-.->|"Optional preview"| Worker
    Runner -.->|"Replay or probe report"| Worker
    Worker --> Store --> Dashboard
    Dashboard -.->|"Engineer prepares a test"| Runner

    classDef workflow fill:#f0f3f7,stroke:#8293ab,color:#253448
    classDef analysis fill:#e9eff8,stroke:#6b83a5,color:#182c45
    class App,Runner workflow
    class Worker,Store,Dashboard analysis
```

The exporter sends OTLP/HTTP asynchronously. The receiver validates the batch and processes accepted work on a bounded worker. SQLite stores normalized calls, evidence references and grouped observations. The dashboard and API read from the same store. The application retains control of inference and tool execution.

A request builder can call preflight before dispatch. A separate runner can submit baseline and edited trials for comparison. Only the provider or application can supply evidence about inputs that were actually assembled.

[Capture and deployment](../DEPLOYMENT.md) · [Evidence contracts](../schema)
