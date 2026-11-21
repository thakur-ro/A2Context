# Fresh questions for the shared-reference recommendation

Freeze the Rust detector before sending these five questions. Start an empty sidecar and the same Collector retrieval application, pinned corpus and local model as the first Collector pilot. Run the questions once, in this order. This tests new questions on a familiar workflow. All five questions count, including failures and silent sessions.

1. If an export fails briefly, which exporter helper settings control how long the Collector retries it?
2. Can I configure the batch processor to group spans by tenant metadata, and what limits the number of groups?
3. What URL paths receive traces, metrics and logs on the HTTP receiver?
4. How should I place the memory limiter in a pipeline with a batch processor?
5. Which OTLP exporter settings would I use to configure a backend endpoint and TLS?

Record the first displayed recommendation, wall time from capture start, how many distinct recommendations appeared, and the time of developer review. Independently compare the recorded answer and application source field with the labels on the retrieved passages. Count a recommendation once even when different replies repeat it. A finding is actionable here if the application serves at least one cited identifier from the prompt examples that fails to label a retrieved passage. Retain legitimate references alongside any stray identifiers. Report silent cases and model or exporter failures separately.
