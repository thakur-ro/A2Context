# NexusWeave guide

Run the commands in this guide from the repository root.

NexusWeave helps you investigate which inputs changed an LLM workflow decision. Start from a recorded tool choice, argument, or answer. Inspect the prompt sources and tool results on that call, then replay a targeted edit to measure how often the decision changes.

Traces show what was recorded. Conflict checks use declared rules, and replay effects describe changes under the tested conditions. Compare those findings across turns, threads, users, or organizations when the trace carries the corresponding identifiers. NexusWeave accepts OTLP, LangSmith exports, and its own versioned JSON format. ContextCite provides a related attribution method for generated statements. [ContextCite paper](https://arxiv.org/abs/2409.00729)

## Try a decision replay

```sh
cargo run --locked -- serve --replay examples/local-mlx/report.json examples/local-mlx/evidence.json
```

Open the turn evidence and expand **Replay results**. In the saved local-model trials, the baseline request yielded a 30-day return window in 3 of 3 runs. Withholding the returns policy changed all three answers to 60 days. Withholding the store name preserved the 30-day answer. The replay case supplies the baseline request. The inspector reads the saved trials locally. [Open the inspector](http://127.0.0.1:8040)

## Run the local demo

With a Rust toolchain installed:

```sh
cargo run --locked -- serve fixtures/conversation.json fixtures/support.json
```

Open the local inspector, choose **Return a jacket**, and select the evidence on either turn. Switch the second turn's **Decision or step** selector between the answer and the order lookup. **Open decision details** opens the full inspector for that step. **Back to conversation** returns to the selected turn. [Open the inspector](http://127.0.0.1:8040)

**Conversation** groups consecutive captured user messages in an identified turn when they have adjacent positions in the same message stream. Gaps, role changes, and unavailable messages end the group. Calls without a turn ID show the newest captured user message and suppress repeats across calls in the same trace.

**Timeline** shows the session in call order. **Patterns** groups findings across imported runs, an organization, or a user within an organization. Each occurrence opens the call and its excerpts.

The fixtures cover conflicting return windows, a holiday exception, duplicated instructions, and a redacted source. Two organizations share the same example user ID to demonstrate scoped grouping. All fixture data is synthetic.

On the chronological **Timeline**, expand a call's findings to open the exact rule pair and decision. **Patterns → Conflicts and duplicates** groups findings by rule and source version. Filter by organization, user, or thread. From a finding, **View finding across threads** opens its group across the imports. Choose **Open turn** beside an occurrence to read the conversation at that decision.

## Share a view or a piece of evidence

Use **Copy view link** to share the selected conversation turn, decision, timeline, pattern scope and filters, or comparison. On a context entry, finding, retrieved document, or replay, **Copy link** opens the exact evidence. The receiving inspector needs the referenced traces and replay reports loaded. If it uses a different local address, paste the link's fragment after that inspector's address.

Links use recorded IDs and scope in the URL fragment. Pattern links also carry the current filter text. The inspector resolves those IDs against local evidence when the link opens. **Refresh traces** uses the same mechanism to reopen your current view after loading new traces.

## Group recurring conflicts

Open **Patterns → Conflict breakdown**. Choose an organization or a user within an organization. Pick a field under **Group by** and use **Then by** to add another. A rule key narrows the results. From a conflict in the transcript, **Find recurring conflicts** opens a breakdown for that rule and the recorded user or organization.

Select a group to see the conversations that contributed to it. **Open turn** takes you to the exact decision. Decisions with missing conversation IDs appear by trace and count separately from identified conversations.

The view counts each decision once in the scope totals. **Scoped** decisions declare a rule and its applicability. **Comparable** decisions have at least two captured values for that rule. **Conflicts** count decisions where a pair of those values disagrees. A source group includes decisions that carried that source and version. Its comparable count requires a captured value from that source, and its conflict count requires that value to disagree with another applicable value. One decision may contribute to several source groups.

The browser and CLI use the same [versioned conflict view](../schema/conflict-view-1.json). Save a view as `data/conflicts.json`, then run `cargo run --locked -- query-conflicts data/conflicts.json fixtures/support.json`. The local API accepts the same object as JSON at `POST /api/conflict-query`.

```json
{
  "version": 1,
  "scope": {"kind": "user", "organization_id": "demo-retail", "user_id": "user-042"},
  "group_by": ["source_version", "rule"],
  "filters": {"rule": "return_window_days"},
  "limit": 20
}
```

The response includes group keys and coverage counts. Copy a group's `key` into the request's `group` field to page through its decision references. Use the returned `next_offset` for the next page. The API accepts up to three distinct grouping fields. Optional RFC 3339 `filters.after` and `filters.before` form an inclusive-start, exclusive-end window. Day groups use UTC. A time filter includes calls with timestamps inside its window, and an unfiltered day grouping shows calls with unavailable times in a separate group.

## Connect an MCP client

Build with `cargo build --locked` and point your MCP client at the resulting binary using the stdio transport. Set the command and arguments in the client's server configuration. Use absolute paths for the binary and evidence files.

```json
{
  "command": "/path/to/NexusWeave/target/debug/nexusweave",
  "args": ["mcp-stdio", "/path/to/NexusWeave/fixtures/conversation.json"]
}
```

The client starts a local process that loads the named evidence files once. To restrict a process to one organization or user, add `--scope /path/to/scope.json` before the evidence paths. The file contains the same scope object used in a conflict view. For example, `{"kind":"organization","organization_id":"Northland Retail"}` limits both queries and direct evidence lookups to that organization's runs, including when the agent requests workspace-wide results.

Use `describe_dimensions` to discover allowed grouping fields and coverage definitions. Submit the same [conflict view](../schema/conflict-view-1.json) to `query_conflicts` that you would pass to the CLI. Copy a returned group key into the next query to get contributing decisions. Pass their run, call, and decision IDs to [`get_turn_evidence`](../schema/turn-evidence-request-1.json) to read findings, source and tool links, and the sequence of recorded calls. If a call has several decisions, omit the decision ID to list them before selecting one. Follow the returned `inspector_path` on a local browser inspector serving the same evidence. The result includes structured JSON and a text form for clients that consume tool text.

By default, turn results include source names, locations, and coverage while omitting captured input and outcome text. To make excerpts available, start the process with `--include-excerpts` and set `include_excerpts: true` in a turn request. Each excerpt is capped at 600 characters and marked if truncated. Add `--replay /path/to/report.json` before the evidence files to include validated replay counts and measured effects. The server reads reports and serves references to recorded inputs. Original request bodies, replay trial bodies, and runner execution stay in the local inspector and CLI workflows.

The stdio adapter uses the official Rust MCP SDK. It serves both the 2025-11-25 initialization flow and the 2026-07-28 discovery flow. [MCP versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)

Hosts with MCP Apps support can render `query_conflicts` inline. Select a group, open a contributing conversation decision, and inspect its findings and source anchors. When the host enables excerpts for the process, the embedded view offers **Show captured text**. The view calls the same read-only tools through the host. Clients using tool text or structured results can follow the same drill-down with tool calls. [MCP Apps quickstart](https://apps.extensions.modelcontextprotocol.io/api/documents/quickstart.html)

### Follow live traces through MCP

Start the [local OTLP receiver](#receive-live-opentelemetry-traces), then give the MCP client the same store directory. The client can start before any traces arrive.

```json
{
  "command": "/path/to/NexusWeave/target/debug/nexusweave",
  "args": ["mcp-stdio", "--otlp-store", "/path/to/NexusWeave/data/live-otlp"]
}
```

MCP reads newly saved batches and activation reports when a tool runs. It opens the journal in read-only mode while the inspector continues receiving data. Measurement tools are discoverable from startup. A query can return no measurements until a report arrives. Scope and excerpt settings apply to live data as well. If the inspector also loads separate evidence files, add those paths to the MCP configuration to query them and any reports attached to their decisions.

Try the [saved Qwen experiment](#attach-activation-measurements-after-a-trace-arrives) while the MCP client is running. Query `query_activation_effects` in `qwen-demo-org`. Open a contributing `location` with `get_turn_evidence`, then use that location and its report ID to read the affected tokens or continuations. Tool results include paths to the same checkpoints in the browser inspector.

### Query activation effects through MCP

Add `--activation-report` and the report path before the evidence files in the MCP configuration. A report array works too. When the configured scope contains a measured answer timeline, the server exposes [`query_activation_effects`](../schema/activation-effects-query-1.json).

```json
{
  "version": 1,
  "block": 0,
  "scope": {
    "kind": "user",
    "organization_id": "demo-retail-north",
    "user_id": "customer-01"
  },
  "source_id": "returns-policy",
  "checkpoint": "Refund timing"
}
```

Use this request with the [six-conversation cohort](#find-repeated-activation-effects-across-conversations). The group summary counts distinct identified conversations as well as decisions and measurements. Add a returned `key` as `group` to page through the individual measurements. Each row includes the original and patched log odds, the turn location for `get_turn_evidence`, and local inspector paths to the exact checkpoint and source. Use the same scope, block, and filters on each page. A `thread` filter takes the trace source, organization, user, and conversation ID from an occurrence. Use RFC 3339 `after` and `before` to select when calls started. The start is inclusive, the end is exclusive, and a time filter excludes calls without a captured start. All queries remain within the process's configured `--scope`.

MCP Apps hosts can show the query inline. Change the decoder block, open a group, and then open a contributing turn. The default response contains measurements and references. The host must start the process with `--include-excerpts` before a request can use `include_excerpts: true` or the inline **Show tested values** button. This opt-in returns bounded edit text and token contrasts.

### Inspect masked-source patterns in an MCP app

Configure the evidence and report files from the [named-output example](#follow-a-named-output-across-different-answers) in your MCP server arguments. Call [`query_visibility_patterns`](../schema/visibility-pattern-query-1.json) with `{"version":1,"kind":"pair","target":"returns.refund_duration","source_id":"refund-policy-copy"}`. A host supporting MCP Apps can render the result inline. Open a group, optionally group it by UTC period, and choose **Inspect token** on a contributing turn. The app shows the individual and joint effects for that pair. It also offers the recorded turn and the paths to its local inspector and sources. **Show token text** appears when the MCP process allows excerpts. [MCP Apps specification](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)

For the same drill-down through tool calls, pass a pattern occurrence's `location` and `report_id` to [`query_source_visibility`](../schema/source-visibility-query-1.json). Use `view.kind` of `sources` or `pairs` with the occurrence's zero-based `output_token`. Supply `view.context_id` for one source or `view.context_ids` for an exact pair. These filters work even when the source or pair would appear on a later page of the ranked results.

## Import public retail traces

Python dependencies are needed only to prepare the sample. Install `huggingface_hub` and `pyarrow` in a Python environment, then run:

```sh
python scripts/sample_exgentic.py
cargo run --locked -- import data/exgentic-retail.jsonl data/exgentic-evidence.json
cargo run --locked -- annotate data/exgentic-evidence.json examples/retail-annotations.json data/retail-annotated.json
cargo run --locked -- serve data/retail-annotated.json fixtures/support.json
```

The sampler pins the [Exgentic dataset](https://huggingface.co/datasets/Exgentic/agent-llm-traces) revision and selects the first eight distinct retail sessions in shard and row order with a linked tool result. It records the revision, session IDs, and content digest in a local manifest. The CLI writes normalized evidence to a new file. Public data stays in the ignored `data/` directory.

Public traces provide recorded model inputs and outputs, tool definitions, and message parts. Rule analysis uses explicit annotations in the normalized JSON. The public dataset supplies session IDs, so user and organization fields remain empty.

### Reference comparison

The [sample manifest](../examples/retail-sample.manifest.json) identifies the eight verified sessions. They normalized to 99 model calls and 91 tool-call/result links. The annotated demo opens the email-based identity lookup in the first session. The investigator can read the applicable policy excerpt, inspect the invocation, and open the later call that received its result. The excerpt is a human annotation of the recorded input.

Reproduce the baseline with `agentpile==0.3.1` installed:

```sh
python scripts/view_retail_baseline.py data/exgentic-retail.jsonl
python scripts/view_retail_baseline.py data/exgentic-retail.jsonl --transcript
```

The first command wraps the original spans in an OTLP envelope. Agentpile retains the spans and reports an unsupported framework dialect, with no mapped steps. The second displays the last captured request and response as a transcript. In that view, search for the identity lookup, then search the message history for the policy. The transcript preserves those excerpts but omits the original call boundaries. NexusWeave presents the annotation beside the observed action. Its repeated-finding drill-down uses the synthetic cohort because the public sample has no joined user identities or labeled policy conflicts. This comparison covers one session and was untimed.

## Import LangSmith traces

Save the [LangSmith run objects](https://docs.langchain.com/langsmith/run-data-format) for the traces you want to inspect. Include the root and child spans to retain thread metadata and tool evidence. Pass a JSON array, a JSON object with a `runs` array, or one run object per line. Try the import with the synthetic example:

```sh
mkdir -p data
cargo run --locked -- import-langsmith fixtures/langsmith.json data/langsmith-evidence.json
cargo run --locked -- serve data/langsmith-evidence.json
```

The command reads the export locally and uses the same evidence format as the retail importer.

Open **Timeline** and choose `retail-demo-thread` to inspect two traces together. The importer groups the export by trace ID and reads the conversation identifier from `extra.metadata.thread_id` or `extra.metadata.session_id`. LangSmith uses the top-level `session_id` for the tracing project. `organization_id`, `user_id`, and `turn_id` require explicit application metadata. Conflicting identities within a trace fail the import.

In **Conversation**, a declared `turn_id` lets adjacent user messages appear together. The inspector reads their positions from the captured LangSmith content paths for flat message lists and single-prompt batches. The exact paths remain available for source evidence and annotations.

The importer reads recorded prompts, chat messages, model generations and choices, and tool spans. It links tool results by explicit tool call ID. An export with missing or masked inputs or outputs can show the span while leaving its context or outcome unavailable. Export one prompt per run for batched requests. Application metadata can carry [context declarations and the original inference request](#declare-context-sources-and-decision-scope).

## Import OpenTelemetry traces

Point `import-otlp` at a saved [OTLP/JSON trace export](https://opentelemetry.io/docs/specs/otlp/#otlphttp-request) or an OpenTelemetry Collector file exporter’s JSONL output. Try the synthetic inventory trace:

```sh
mkdir -p data
cargo run --locked -- import-otlp fixtures/otlp.json data/otlp-evidence.json
cargo run --locked -- serve data/otlp-evidence.json
```

Select **Timeline** and choose `inventory-demo-thread`. The import reads GenAI chat, generation, completion, and tool-execution spans. It keeps trace and span IDs, the original attribute names, and nanosecond timestamps. Tool results link by explicit `gen_ai.tool.call.id`, and a tool execution stays distinct from the context sent on the later inference. A message reference appears as unavailable context with its recorded reference.

Use `gen_ai.conversation.id` to group traces into conversations. Add `nexusweave.organization.id` and `nexusweave.user.id` on spans or resources for explicit identity grouping, and `nexusweave.turn.id` on spans for turn counts. OTLP exports can omit captured messages, tool arguments, and results. The inspector still shows supported spans with the evidence that was recorded.

### Import OpenInference spans

The same OTLP importer and live endpoint accept OpenInference `LLM`, `TOOL`, `DECISION`, `GUARDRAIL`, and `RETRIEVER` spans. Chat messages, text parts, tool definitions, and tool choices retain their exact flattened attribute paths. Tool results link by explicit call IDs. OpenInference `session.id` and `user.id` supply fallback conversation and user identifiers. When both conventions describe the same call, the importer uses the GenAI evidence when present and falls back to OpenInference when GenAI supplies only operation metadata. [OpenInference semantic conventions](https://arize-ai.github.io/openinference/spec/attributes/semantic-conventions.html)

```sh
mkdir -p data
cargo run --locked -- import-otlp fixtures/openinference.json data/openinference-evidence.json
cargo run --locked -- serve data/openinference-evidence.json
```

Open **Timeline** and choose `openinference-inventory` to follow a synthetic workflow from the routing decision through the tool result and guardrail. Structured decision and guardrail responses retain their recorded JSON. For JSON requests, top-level fields appear as context with anchors such as `input.value#/state`. The fixture opts into original request capture on its routing span, so you can replay a change to `/state` and score just the `/route` outcome field. [OpenInference decision spans](https://raw.githubusercontent.com/Arize-ai/openinference/main/spec/decision_spans.md)

### Follow the recorded execution tree

For threads with parent span data, open **Timeline → View → Execution tree**. The tree shows the recorded parent of each call, including wrapper spans that have no normalized decision. Select an inspectable span to open its context and outcome. **Show in execution tree** on the call goes back to that span. The OpenInference fixture includes separate **Stock branch** and **Response checks** spans.

The OTLP importer uses `parentSpanId`, and the LangSmith importer uses `parent_run_id`. Each parent is resolved within its trace. A missing parent stays visible by ID. **Refresh traces** keeps the current timeline open and reconnects parents recorded in late batches. The chronological view is available for traces without parent data. [OpenTelemetry trace specification](https://opentelemetry.io/docs/specs/otel/trace/api/), [LangSmith run format](https://docs.langchain.com/langsmith/run-data-format)

### Follow retrieved documents into model inputs

```sh
mkdir -p data
cargo run --locked -- import-otlp fixtures/retrieval.json data/retrieval-evidence.json
cargo run --locked -- serve data/retrieval-evidence.json
```

In **Timeline**, select `returns-retrieval` and open **Search returns policies**. The example captures two search results. `policy-current` links to its exact input on the model call, and `policy-obsolete` has no recorded input match. **Patterns → Retrieval handoffs** counts captured and matched documents within a thread, user, or organization. [OpenInference document conventions](https://arize-ai.github.io/openinference/spec/attributes/semantic-conventions.html#document)

A match requires the same captured document ID and text, an earlier retrieval completion time, and a unique eligible retrieval within the trace. Changed or unavailable text, unknown timing, and multiple matches surface as separate statuses. The importer reads OpenInference `retrieval.documents`, document parts in `llm.input_messages`, and `input.documents` on LLM spans. The `document_handoffs` array from `inspect` preserves both locations. This evidence traces an input to a matching retrieval. Changing that input in a replay tests its effect on the outcome. [OpenInference message content](https://arize-ai.github.io/openinference/spec/content/messages.html)

### Receive live OpenTelemetry traces

Start a local receiver and inspector with a dedicated store directory:

```sh
cargo run --locked -- serve --otlp-store data/live-otlp
```

Point an SDK or Collector's OTLP/HTTP trace exporter at `http://127.0.0.1:8040/v1/traces`. The receiver accepts binary Protobuf and OTLP JSON, with optional gzip compression. Use an [OTLP HTTP exporter](https://opentelemetry.io/docs/specs/otlp/#otlphttp) for traces.

```sh
export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:8040/v1/traces
export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf
```

To try it without an instrumented workflow, send the saved JSON fixture:

```sh
curl -fsS -H 'Content-Type: application/json' \
  --data-binary @fixtures/otlp.json http://127.0.0.1:8040/v1/traces
```

The [Python SDK example](../examples/send_otlp_inventory.py) sends a synthetic tool choice, tool result, and model answer as separate gzip-compressed exports. Install its exporter and run it in the same Python environment. [OpenTelemetry Python exporters](https://opentelemetry.io/docs/languages/python/exporters/)

```sh
python3 -m pip install opentelemetry-sdk opentelemetry-exporter-otlp-proto-http
python3 examples/send_otlp_inventory.py
```

Open the inspector and select **Refresh traces**, then choose `sdk-inventory-thread` in **Timeline**. Accepted batches persist as numbered JSON files in the store. Spans are merged by trace and span IDs, sorted by recorded timestamps where available, and retained across restarts. Identical retries count once. The receiver accepts batches up to 64 MiB before and after decompression.

You can also pass evidence files to `serve` alongside the store when they contain different trace IDs. Live intake and the inspector bind to loopback.

### Attach activation measurements after a trace arrives

Start `serve` with `--otlp-store` and export the inference span first. POST one [activation timeline](../schema/activation-timeline-2.json) or [source visibility report](../schema/source-visibility-1.json) to the local inspector. Use the run, call, decision, and context IDs from `/api/inspection`. For an OTLP GenAI text or tool output, the run ID is `otlp:<trace-id>`, the call ID is the span ID, and the decision ID is `output:<message-index>:<part-index>`. Both indices start at zero.

```sh
curl -fsS -H 'Content-Type: application/json' \
  --data-binary @activation-report.json http://127.0.0.1:8040/api/activation-reports
```

The inspector checks the report against the recorded decision and captured sources. It returns `201` when the report is added, `200` for the same report again, and `400` for an invalid report. A `409` with an unavailable-decision message means the trace must arrive before the report can be retried. A `409` for a conflicting report means the same ID at that decision already holds different measurements. The endpoint accepts up to 16 MiB of uncompressed JSON per report.

Choose **Refresh traces** to see the report on its turn and in **Patterns**. Reports are stored under `activation-reports/` inside the OTLP store and restored on restart. You can also attach a report to a decision loaded from an evidence file. Supply that evidence file again after restart to view its report.

Use the [OpenTelemetry SDK example](../examples/send_otlp_activation.py) to send the trace for a saved Qwen experiment and the measurements from that run. It prints a link to the turn. Running the example again with another conversation ID puts both conversations under the same demo user and organization.

```sh
python3 examples/send_otlp_activation.py
python3 examples/send_otlp_activation.py --conversation-id qwen-measurement-second-thread
```

### Query measurements from a running inspector

Use the same request schemas as the MCP tools. These routes accept HTTP `POST` with `Content-Type: application/json` on the local inspector. They query the currently loaded evidence and reports, including live arrivals. Excerpts are optional with `include_excerpts: true`. Group keys and offsets work the same way as in MCP.

| Route | What it returns |
| --- | --- |
| [`/api/activation-effects`](../schema/activation-effects-query-1.json) | Comparable checkpoint effects by organization, user, thread, or time window. Add a group key to get the contributing decisions and source links. |
| [`/api/turn-evidence`](../schema/turn-evidence-request-1.json) | Recorded call order, context links, conflicts, and available measurement report IDs for a turn. |
| [`/api/output-token-effects`](../schema/output-token-effects-request-1.json) | Effects of source interventions on each recorded token at one decoder block. |
| [`/api/generation-branches`](../schema/generation-branches-request-1.json) | Measured continuations after an activation swap, including the zero-based index of the first token that differs from the recorded output. |
| [`/api/source-visibility`](../schema/source-visibility-query-1.json) | A timeline of measured attention-mask effects, ranked source effects at a token, or paired-mask comparisons at that token. |
| [`/api/visibility-patterns`](../schema/visibility-pattern-query-1.json) | Recurring source effects or pair interactions at a matching output token, with links to each contributing decision. |
| [`/api/context-sources`](../schema/context-source-query-1.json) | Named source coverage, decision links, and example replay or mask measurements. |

```sh
curl -fsS http://127.0.0.1:8040/api/activation-effects \
  -H 'Content-Type: application/json' \
  --data '{"version":1,"block":0,"scope":{"kind":"organization","organization_id":"qwen-demo-org"}}'
```

To follow a group into a turn, send the same request with `group` set to its `key`. Pass an occurrence's `location` to `/api/turn-evidence`, or pass `location` and `report_id` to `/api/generation-branches`. Keep the same query filters when paging. New arrivals may change the contents and counts between pages.

### Capture the request for a controlled replay

Set the optional `nexusweave.request.body` string attribute on the inference span to the JSON body sent to the model. This NexusWeave extension works with both live exports and saved OTLP imports. If the exporter truncates the JSON, the trace stays available and request verification is unavailable. The body can include prompts, tool results, and personal data. Keep transport credentials and headers out of the attribute. [OpenTelemetry guidance on capturing GenAI content](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md#recording-content-on-attributes)

```sh
python3 examples/send_otlp_inventory.py --capture-request
```

Refresh the inspector, select `sdk-inventory-captured-thread`, and open the first call. In **Plan replay**, choose **Use captured request**. Use **Download call evidence** to save the selected call as a standalone evidence file. Download the case after selecting an intervention. Pass both files to the `replay` command with your runner. To view the resulting report on the original trace, start the inspector with its original store and add `--replay <report.json>`.

## Declare context sources and decision scope

An application can use `nexusweave.context.semantics` to label captured context with a stable source ID, version, or rule. It can also declare which rule scope a recorded decision addresses and which inputs apply to it. Send the declaration as a JSON string attribute on the OpenTelemetry inference span, or as a JSON object or string under `extra.metadata` on a LangSmith LLM run. [Declaration schema](../schema/context-semantics-1.json)

```json
{
  "version": 1,
  "context": [
    {
      "anchor": {"field": "gen_ai.system_instructions", "part_index": 0},
      "source_id": "retail/returns-policy",
      "source_version": "v12",
      "rule": {"key": "return_window_days", "scope": "standard", "value": 30}
    }
  ],
  "decisions": [
    {
      "anchor": {"field": "gen_ai.output.messages", "message_index": 0, "part_index": 0},
      "applies_to": {"key": "return_window_days", "scope": "standard"}
    }
  ],
  "relationships": [
    {
      "context": {"field": "gen_ai.system_instructions", "part_index": 0},
      "decision": {"field": "gen_ai.output.messages", "message_index": 0, "part_index": 0},
      "relation": "applies to"
    }
  ]
}
```

An anchor uses the exact `field`, `message_index`, and `part_index` shown in the normalized evidence. An omitted index selects evidence with no index at that position. LangSmith anchors can use paths such as `inputs.messages[0].content` and `outputs.output`. Use the paths in the imported trace because serialized message layouts vary.

For LangSmith, also set `extra.metadata["nexusweave.span.id"]` to the run's exact `id` so inherited metadata binds only to its intended call. The same run binding enables opt-in capture of `extra.metadata["nexusweave.request.body"]` as a JSON string for verified replays. Configure capture where the application has the actual request body. [LangSmith trace metadata](https://docs.langchain.com/langsmith/add-metadata-tags), [LangSmith Python SDK inheritance](https://github.com/langchain-ai/langsmith-sdk/blob/main/python/langsmith/run_helpers.py)

Each declaration binds within its own call. If an anchor is missing or ambiguous, the inspector keeps the original trace and shows the error under **Context on this call**. Explicit relationships appear as **declared**. Two captured rules with the same key and scope can produce a conflict or duplicate finding for a decision in that scope. Use the same source IDs and versions across integrations to aggregate recurring rule findings.

If several sources are assembled into one prompt string, use `request_sources` to identify each source in `nexusweave.request.body`. NexusWeave extracts the text and adds a context item with the request's evidence. For example, if `/messages/0/content` starts with `Préface\nUse 30 days.\n`, this declaration captures `Use 30 days.`.

```json
{
  "version": 1,
  "request_sources": [
    {
      "id": "policy",
      "source_id": "retail/returns-policy",
      "source_version": "v12",
      "role": "system",
      "request_fragment": {
        "pointer": "/messages/0/content",
        "text_range": {"start_byte": 9, "end_byte": 21}
      }
    }
  ],
  "relationships": [
    {
      "context": {"request_source_id": "policy"},
      "decision": {"field": "gen_ai.output.messages", "message_index": 0, "part_index": 0},
      "relation": "applies to"
    }
  ]
}
```

Each `id` is unique within its call. Use a stable `source_id` across calls. Offsets count UTF-8 bytes in the decoded request string from zero. The end offset is exclusive. An invalid range or missing request body leaves the original trace available and reports a declaration error. If source text is already captured separately, add `request_fragment` to its anchored declaration or normalized context item. In that form the inspector verifies that the range matches the captured text.

Run `python3 examples/send_otlp_inventory.py --request-sources` for a single system prompt with two declared sources. In **Decisions**, open the tool choice in `sdk-inventory-prompt-sources`, then expand **Prompt source layout** to inspect ordering and overlapping ranges. Selecting a source opens its exact context entry, where you can prepare a replay.

The SDK example declares `inventory-lookup-tool` version `v1` on the tool selection call. Send it with two different `--conversation-id` values, then open **Patterns → Context links** and search for that source. The declared link opens the original tool choice and its metadata in either conversation.

## Emit evidence from another workflow

The [JSON Schema](../schema/evidence-1.json) defines an array of versioned run records. A run contains ordered calls, with separate entries for context, decisions, and tool results. See [the synthetic fixture](../fixtures/support.json) for a complete example. Any producer can emit this JSON.

- Use stable run IDs and call-local IDs for context items and decisions. Set `evidence` to the original field and optional span ID and zero-based message and part indices.
- For recorded trace topology, supply the optional run-level `spans` array with span IDs, parent span IDs, and optional names and start times. A call's `span_id` matches its span within the same run.
- To track a document through a trace, include `document_identity` with the external ID and its evidence on the input context item and on the corresponding item in the originating call's `retrieval.documents` list. Provide the retrieval's `ended_at` and the consuming call's `started_at`.
- Supply rule `key`, `scope`, and `value` only when these are known. The analyzer compares pairs with the same key and scope as the selected decision. Omitted or redacted values yield insufficient evidence.
- Supply stable source IDs and versions for repeated-pattern grouping. Use explicit organization and user identifiers to scope identity. A turn count requires a conversation ID and turn ID.
- To enable verification of replay baselines, add `recorded_request` to the call with the original JSON body as a string in `body` and its field or span in `evidence`.
- Add a relationship to connect a decision to an item in its context. State the relation and its basis: observed, declared, derived, or hypothesis. Observed relationships require an evidence reference.
- Use `nexusweave validate <file>` to check a record set or `nexusweave inspect <file>` to print evidence and findings as JSON.

The analyzer matches the tool invocation ID to a result recorded on a later call in the same run. It records observed outcomes and checks explicit scope. A trace on its own leaves the influence of any particular input unknown.

## Inspect recorded context matches

The decision panel links a recorded tool name to matching tool definitions on that call. It also finds exact argument values in recorded tool results or identifiers in user text. Where an earlier tool invocation has a unique matching call ID, the panel can open that invocation. Exact matches are candidates for replay. Causal attribution requires testing an intervention.

For model answers, routing decisions, and guardrail outputs, NexusWeave can link a structured field to the same field name and value in structured context. A whole numeric answer links only when exactly one typed field matches. In prose, the inspector links distinctive identifiers such as `MUG-42` from structured context. An identifier match highlights the shared identifier. A replay can then test whether that source affected the answer.

In **Patterns**, choose **Tool calls** to group recorded invocations by tool and trace source. Filter by name or switch to an explicit user or organization. Open any occurrence to inspect its decision and matched context.

### Investigate an unfamiliar decision

Open **Investigate context** on a recorded choice or answer. The first view shows inputs with exact matches, declared relationships, rule findings, or saved replays. Use **All recorded inputs** and search the captured text when the trace has few links. Selecting an input opens its original evidence. For a named input, **View source history** shows where that source recurs across threads, users, and organizations. The replay column shows which inputs match the original request, where you need to choose a field, and when you need a separately saved request.

Try the [return-window demo](#try-a-decision-replay) for an answer and the [local inventory demo](#replay-structured-context) for a tool choice. Search for `30 days` in the return-window case to find the policy in the recorded prompt. The inventory case links the recorded tool name to its captured definition. Both demos include saved local-model trials. Public trace imports also show the capture gaps that prevent a verified replay.

## Find recurring context links

Choose **Patterns → Context links** to see how often a captured source was linked to the same decision. Filter across all imports, one organization, one user, or one thread. **Timeline** puts the links on the associated model calls. Open a link to reach its exact excerpt or annotation. For a matched tool result, the inspector opens the earlier invocation when a unique call ID resolves it.

Change **View** to **Source–decision matrix** to see which decisions share a context source. Each cell counts distinct recorded decisions and shows how many linked context items have an individual replay measurement. Select a cell to see its relationships and open their original evidence. Use the existing scope and search controls to narrow the matrix. The replay filter selects the links shown in the drill-down while the cell totals describe the links in scope.

Choose **Ready to plan, unmeasured** to find linked inputs with an available decision outcome and an exact match in the captured request. Open a link group and choose **Plan replay** to preselect the input. **Replay planning gaps** shows the remaining unmeasured inputs and gives the reason on each link, such as a missing original request or a captured value with no matching field. The coverage totals count an input once per decision even if several relationships point to it. An earlier measurement still counts when the original body is unavailable.

The public retail sample has six threads where a name-and-ZIP lookup result matches the `/user_id` argument of a later account-details call. Search for `find_user_id_by_name_zip` and open the group whose source is the lookup result. Try the `returns-policy` group in the synthetic demo to see how the same declared relationship repeats across users and organizations. Public traces in the sample have thread IDs. The user and organization filters rely on explicit identifiers in the evidence.

Groups keep the decision kind, label, trace source, and relationship basis. Exact argument and outcome field matches retain the output path and the path to the matched input value when available. Tool results with a resolved origin group by the producing tool, so different customer or order IDs can appear in the same group. Tool definitions without a source ID group by their normalized JSON. Named sources use their supplied version or a text digest when only a name is available.

## Inspect capture coverage

**Patterns → Capture coverage** lists the context entries recorded on each call and whether the original request was captured. Filter by thread, user, or organization to find unavailable requests, redacted or referenced entries, or entries with zero or multiple exact field matches. Named sources with the same gap on multiple calls link to the affected entries. **Decisions**, the chronological **Timeline**, and **Compare** show the same inventory for a call.

Open **Investigate context** on a turn in **Conversation** or **Decisions** to see recorded links, rule findings, and measurements beside each input. When a source mask report is loaded, **Measured inputs** includes the inputs measured by that probe. Each mask or pair button opens the token with the largest absolute effect for that input in its report. The single-source and paired effects stay labeled separately from replay results.

**Patterns → Context sources** shows how many recorded occurrences have a measured replay or a source mask in the selected workspace, organization, user, or thread. Open a source's occurrences to see which calls have each kind of measurement and jump to those results. A single occurrence can count in both methods.

Choose **Compare with** to compare recorded exposure and measurement coverage side by side. Select the same scope twice and set two **Date range** windows to see how source coverage changed over time. The view aligns sources by declared name and shows each side's recorded versions. A source found in only one scope remains visible. **Copy view link** preserves the scopes, dates, and selected source.

The browser uses the same query as [`query_context_sources`](../schema/context-source-query-1.json) in MCP and `POST /api/context-sources` in HTTP. Omit `source_id` to list named sources, ordered by distinct calls. Pass a source ID to read its occurrences in chronological order. Use `source_version`, `scope`, `thread`, and an inclusive `after` or exclusive `before` to narrow the results.

```json
{
  "version": 1,
  "source_id": "refund-policy-copy",
  "scope": {"kind": "organization", "organization_id": "retail-north"}
}
```

Each occurrence is one captured input on one call, even if several decisions link to or measure it. The result includes the captured input path, linked decision IDs, and counts of individual mask and replay measurements. Each method includes one example link when measurements are available. A replay with a measured zero change counts as measured. The tool returns source metadata and references, while captured text is available through the separate turn evidence query under the host's excerpt settings.

Pass a second scope or time window in `compare`. A missing side is returned as `null`. Use `offset` and `compare_offset` to page through a selected source's occurrences independently. A case-insensitive `search` matches the source name, trace source, or version, and retains both sides when either side matches.

An exact match is a request string, JSON container, or declared text fragment that equals the captured entry. Open **Test this context** on a matched entry to plan an edit. Multiple matches require choosing the field to change. The inventory is scoped to entries present in the trace. The inspection JSON exposes these records under `capture_profiles`. `matching_fields` is omitted when the original request or entry value is unavailable. The browser's inspection response includes the request's evidence anchor. The replay planner loads the original request when you select **Use captured request**.

## Follow a context source across sessions

In **Patterns → Context sources**, search for a named source and filter by organization, user, or thread. The history groups by the supplied source ID across adapters and lists capture availability by version. It includes context items with no decision link. Open an occurrence to reveal its exact context in the original call. From a named item in **Decisions**, **View source history** starts the search across all imported runs.

Try `return-preference` in the support fixture to see two included versions and a redacted version. When one version has multiple captured text forms, the history shows the number of forms and the SHA-256 fingerprint for each available capture. The links count distinct recorded decisions. An item counts as measured when an individual replay produced a usable effect, including a measured zero. **Open replay** links directly to an individual test for that context item. The `source_profiles` array from `nexusweave inspect` includes source IDs, versions, capture availability, and references to the original evidence.

Counts describe recorded matches and annotations. Open the original call to prepare a controlled replay if you have the captured request. The JSON from `nexusweave inspect` includes `context_patterns` with scope fields and references into `context_links` or the call's `relationships`.

## Compare two recorded decisions

Choose **Compare decision** on a call, then select decision B by session, call, and decision. You can compare turns in one session or decisions from different users, organizations, and imports. Each side shows its recorded outcome, model, and findings.

The comparison aligns named sources by their supplied ID. Within a trace it also aligns documents by ID and tool results by recorded invocation ID. Tool definitions from the same trace source align by reported tool name. Other items align by exact captured text with the same kind and role. Repeated matches remain grouped. A source captured on just one side is labeled **Recorded only on A** or **Recorded only on B**. An aligned item with redacted or unknown text shows **Evidence incomplete**.

Use **Differences and gaps** or **With replay tests** to narrow the view. Each item links to its original call and shows the relationships and replay reports for the selected decision. A shared replay setup uses the same tested context, intervention, model, runner, and outcome rule. The natural differences between the two calls give you candidates to investigate with controlled replay.

In the support fixture, compare the first call in `return-eligibility` with the third call in `holiday-order`. The comparison shows the redacted `return-preference` version alongside the available capture.

Load reports with `--replay` to see which linked inputs have measured effects. Coverage counts unique context items on recorded decisions in the selected scope. An item counts as measured when one of its individual replay conditions has a usable result, including a measured zero change. The **Replay evidence** filter narrows the displayed links while the coverage bar continues to describe the selected scope. Open a link to see the trial counts and jump to the exact intervention. Pair results stay labeled as joint tests.

The `context_replays` array in the inspection JSON maps `run_id`, `call_id`, `decision_id`, and `context_id` to zero-based indices in `replay_effects`. A pair appears under both tested contexts with its pair result intact.

## Follow context changes through a thread

In the chronological **Timeline**, expand **Context differences from previous input** to inspect both sides of a change. Timeline uses the same alignment as Compare. It distinguishes version, text, authority, declared request location, and capture status changes. It shows multiple possible matches together. Open either call's context or compare its recorded decisions.

Timeline compares consecutive recorded input calls across retrieval and tool execution steps. A call with no recorded context breaks the comparison on either side. Cross-trace comparisons use RFC 3339 timestamps and require distinct order. Entries recorded on just one side describe the trace's coverage.

Open **Patterns → Context changes** to see where the same change to a named source or advertised tool repeats within or across threads. Groups keep the before and after versions, capture states, and fields that changed. Filter by thread, user, or organization and open a call pair to see its evidence.

Each pattern shows its count alongside the number of compared pairs with the same earlier versions and capture states. A source recorded only on the later call uses every compared pair where the source was recorded on either side. **Inspect compared pairs** shows the observations behind the count, including matching or incomplete captures. The scope summary counts missing context and uncertain order separately.

Open **Decision outcomes across compared pairs** on a pattern to compare recorded outcomes when the pattern appeared and in the other pairs in the same comparison set. Select a count to open those pairs and their decisions. The counts describe associations in the recorded calls. Use [controlled replay](#test-a-context-item-with-a-controlled-replay) to measure the effect of editing a specific input.

Use **Compare with** to see the same patterns side by side for two organizations, users, or threads. Each side has its own qualifying call pairs and decision outcomes. A zero pattern count means qualifying pairs were observed in that scope. **No qualifying pairs** means that scope has no observations satisfying the pattern's comparison criteria. The summary reports how many input call pairs the scopes share. **Copy view link** preserves both scopes and thread selections.

The Timeline summarizes decision changes between ordered calls, including pairs with a missing context snapshot. The inspector groups decisions by recorded kind and label, retaining every decision ID. A change in labels or counts changes the decision mix. Equal-sized groups compare exact outcomes and their counts independent of output order. Missing outcomes are marked incomplete when they could account for the observed differences.

The inspector and `nexusweave inspect` use the same Rust implementation for context changes. The JSON includes these fields.

- `context_timelines` gives the display order of calls for each thread or standalone trace.
- `context_transitions` locates consecutive input calls by run and call ID. A pair has state `compared`, `missing_context`, or `ambiguous_order`. Each compared change lists the context IDs on the before and after sides and names the fields that differ. Ordered pairs include `decision_change`, with status `same`, `mix_changed`, `outcome_changed`, `incomplete`, or `unavailable` and the decision IDs for each group.
- `context_change_patterns` groups comparable changes. Each occurrence records its scope and uses zero-based `transition_index` and `change_index` to locate the exact evidence. The pattern's `source_observation_index` locates its source observations. When present, `earlier_state_index` selects the earlier-state group used for the denominator.
- `context_source_observations` indexes evidence for sources that have a change pattern. Each earlier-state group lists the earlier entries with their versions and capture states. Its `pairs` point to compared transitions and include the source's status and context IDs on both sides.

In **Compare**, structured outcomes and declared rule values are compared using the original JSON values, including full-width integers.

## Test a context item with a controlled replay

A [replay case](../fixtures/replay-case.json) points to a recorded decision and contains the inference request as JSON. Each intervention changes one captured value at a specific JSON Pointer. NexusWeave checks the selected value against recorded context and rotates baseline and variant trial order. When the original body was captured, NexusWeave also compares the full baseline request before sampling and again before displaying a report. The comparison ignores JSON formatting whitespace and preserves member order. The report includes the trial outcomes and the change in the recorded-decision rate, in percentage points. Missing trial outcomes or a baseline that never reproduced the recorded decision leave the effect unavailable.

In **Decisions**, open **Plan replay**, **Test this conflict**, or **Test this context** beside an exact match. Use the recorded body or choose a separately saved JSON request for traces without a captured body. The local inspector uses the same text and JSON matching as capture coverage and preserves full-width integers in the exported request and evidence. If a value occurs more than once, choose the request field to change. The inspector validates the request, intervention, and match rule before downloading a case for the CLI. It labels reports without an independently captured baseline as unverified.

To try the planner with the synthetic demo, save its original request to a separate file:

```sh
mkdir -p data
python3 -c 'import json; print(json.dumps(json.load(open("fixtures/replay-case.json"))["request"]))' > data/support-request.json
```

Choose `data/support-request.json` on the first `return-eligibility` call and use `Synthetic rule runner v1` as the runner ID. Substitute the downloaded file for `fixtures/replay-case.json` in the command below.

```sh
mkdir -p data
cargo run --locked -- replay fixtures/support.json fixtures/replay-case.json data/return-replay.json -- python3 scripts/replay_synthetic.py
cargo run --locked -- serve --replay data/return-replay.json fixtures/support.json
```

The example uses a deterministic rule runner. Replacing the developer instruction changes the return window from 30 to 60 days, and replacing the saved preference leaves the selected window at 30 days. The runner needs one return-window instruction to produce an answer, so withholding both makes the joint result unavailable.

For a model experiment, use a runner that reads one complete request from stdin and returns one JSON object with `kind`, `label`, and `outcome` on stdout. The values should use the same representation as the recorded decision. Configure the runner to send a single inference request using the captured model and settings, and return the proposed action before any tool executes. The runner controls model access and network behavior. A replay measures sensitivity under the tested conditions.

For a declared text fragment, the planner records `operation: "replace_text_fragment"` and the original `text_range` on the intervention. The replay runner receives the request with just that string slice replaced. Two non-overlapping fragments in the same string can be tested separately, together, or as a target and negative control. The report retains both original byte ranges.

### Compare a target edit with a negative control

Select two context items in the replay planner. Under **Negative control**, choose the edited input you expect to leave the decision unchanged. Pick a comparable edit before running trials. A hand-authored case uses `negative_control_context_id` with the ID of that item. The runner samples the original request and each edit separately. The report shows how often each edit reproduced the recorded decision and the difference in percentage points (target edit minus control edit). If the control itself changes the result, the model is sensitive to that edit too. [Negative controls, Lipsitch et al.](https://pmc.ncbi.nlm.nih.gov/articles/PMC3053408/)

The report's `control_comparison` links the contrast to both variants. The contrast is available when every trial in the three conditions can be scored and the baseline reproduces the recorded decision at least once. Independent sampling adds a 95% interval for the two edited conditions. The common baseline cancels in this comparison.

Open **Patterns → Controlled attribution** to compare reports that used the same target and control edits, model label, runner, match rule, and sampling design. The view counts recorded decisions once per comparison in the selected scope. Expand the breakdown to see results by organization, by user within an organization, or by thread for a user. The evidence table shows the baseline, target, and control trial counts for each replay. Select a decision to open its exact comparison, or find it under **Tested inputs and effects** on the call's timeline. The direction counts describe the observed replays, with each decision weighted equally.

Use **Compare with** to align the same replay designs across two scopes. Each side shows its measured comparison counts, match rate changes, and exact trial evidence. The summary counts shared recorded decisions across the two scopes so you can see when they overlap.

The inspection JSON includes these groups under `replay_effects`. They have ordered `target_edit` and `control_edit` fields. Each occurrence has `baseline`, `variant` (the target edit), `control`, and `difference_pp` (target minus control). The individual effects for each edit remain available under **Replay effects** and in `context_replays`.

### Measure replay uncertainty

If the runner draws independent samples within and across conditions, select **Runner uses independent samples** in the planner or set `"sampling_design": "independent"` in the case. Choose the trial count and decision match rule before running the replay. The report records the sampling declaration. Cases without a sampling declaration still report the observed change.

For each measurable change, the inspector shows a conservative 95% interval in percentage points. It uses Hoeffding's inequality with a half-width of `100 × sqrt(k × ln(40) / (2n))`, where `n` is the trial count per condition. A change from baseline uses two conditions and a pair interaction uses four. The endpoints are clipped to the possible range of the effect and rounded outward. The interval applies to a single reported contrast. Rescored reports retain the trials and report the observed change under the new rule. [Hoeffding, 1963](https://doi.org/10.1080/01621459.1963.10500830)

### Choose what counts as a match

Replay matches the recorded kind, label, and full outcome by default. Use **Decision match** in the browser planner or set `match_rule` in a case file when the investigation concerns a specific part of the outcome.

| Rule | What a trial must reproduce | Case field |
| --- | --- | --- |
| Exact decision | Kind, label, and full outcome | Omit `match_rule` |
| Decision kind and label | Kind and label, such as the name of a proposed tool | `{"kind":"kind_and_label"}` |
| Selected JSON fields | Kind, label, and the values at the selected paths | `{"kind":"outcome_fields","pointers":["/sku"]}` |
| Value captured from text | Kind, label, and one value captured from the answer | `{"kind":"text_capture","pattern":"([0-9]+) units"}` |

The CLI checks that the rule can score the recorded outcome before running any trials. JSON field matching accepts objects, arrays, and strings containing encoded JSON. A text rule must have exactly one capture group and extract exactly one nonempty value. If a response with the same kind and label lacks a selected field or a unique capture, its trial is unscored. Unscored or failed trials leave the corresponding effect unavailable. Reports retain the original outcomes for review. [JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901.txt), [Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax)

To compare rules on the same trial outcomes, use `nexusweave rescore-replay <evidence.json> <report.json> <match-rule.json> <new-id> <output.json>`. The command writes a new report with the original trial set identified by `rescored_from`. The rule file contains one JSON object from the table. Use `{"kind":"exact"}` for exact matching. Reports derived this way share their source trials.

In the [inventory quantity experiment](../examples/local-mlx-tool/case-quantity.json), the local model reported 42 units in all six baseline trials. Two answers reproduced the exact recorded wording. With the JSON result replaced, four answers said “0 units” and two said “no units.” The original numeric capture left those last two unscored. The [revised capture](../examples/local-mlx-tool/match-quantity.json) covers both forms. The three reports below use the same raw trial outcomes.

| Match rule | Baseline matches | Replacement matches | Change |
| --- | --- | --- | --- |
| Exact wording | 2 of 6 | 0 of 6 | −33.3 pp |
| Numeric quantity | 6 of 6 | 0 of 4, 2 unscored | Unavailable |
| Quantity including “no units” | 6 of 6 | 0 of 6 | −100 pp |

```sh
cargo run --locked -- serve \
  --replay examples/local-mlx-tool/report-quantity.json \
  --replay examples/local-mlx-tool/report-quantity-exact.json \
  --replay examples/local-mlx-tool/report-quantity-extended.json \
  examples/local-mlx-tool/evidence.json
```

Open `local-model-inventory-quantity` in **Decisions** to inspect the rules and trial outcomes.

### See what the model chose after an edit

Open **Compare trial outcomes** under an individual or joint replay. Each row counts an outcome under the report's match rule. For example, the extended quantity capture puts the different phrasings of “no units” under one captured value. Failed trials and completed trials the rule could not score have their own rows. Counts are out of attempted trials. Open **Example trial** to read a representative response.

The comparison is also available on **Timeline**, in context and finding drill-downs, and on scoped **Replay effects** rows. In **Replay effects**, a recurring alternative appears when it becomes more or less frequent in the same direction on at least two distinct recorded decisions with measurable replays. The view reports directions per decision and links to each contributing test. A decision can contribute to both directions if its saved replays disagree. The counts describe the observed trial sets.

The inspection JSON includes `replay_outcomes`. Each `replay_index` points into `replays`. Condition 0 is baseline. The individual edits follow in the report's variant order, and the joint trial set comes last when present. Counts omit conditions with zero occurrences. Example references use zero-based condition and trial positions. An `alternative_id` identifies the same projected alternative under the same match rule within that inspection response.

### Replay structured context

For a captured tool definition or result that matches a JSON object or array in the original request, choose **Remove JSON field or array entry** or **Replace with JSON**. For hand-authored cases, set the intervention's `operation` to `remove_json` with an empty `replacement` string, or `replace_json` with a JSON-encoded replacement of the same container type. The CLI checks the matched value before running trials. Request object keys retain their captured order, which can affect generation when a runner renders structured values into the prompt. [JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901.txt)

Structured matching requires the whole captured value to match a request field. If a trace stores a result in a wrapper or omits parts of a tool definition, the original request may use a different shape.

The local inventory example uses synthetic data and the same LFM model as the return-policy example. Removing the inventory tool changed the model's proposed action to a text answer in all three trials. Removing an unrelated store-hours tool preserved the inventory lookup. On the inventory answer, **Investigate context** links the recorded SKU to the inventory result. In a separate replay, replacing that result changed the reported quantity from 42 to 0 in all three trials. The runner records proposals and answers without executing tools. [Local inventory evidence](../examples/local-mlx-tool/evidence.json)

```sh
cargo run --locked -- serve \
  --replay examples/local-mlx-tool/report.json \
  --replay examples/local-mlx-tool/report-result.json \
  examples/local-mlx-tool/evidence.json
```

To repeat the experiment on Apple Silicon, use the [saved case](../examples/local-mlx-tool/case.json) or the [tool-result case](../examples/local-mlx-tool/case-result.json) with the included [MLX runner](../examples/local-mlx-tool/runner.py).

### Test two context items together

Select two items in the planner and enable **Also test both changes together**. For hand-authored cases, set `"test_pair": true` with exactly two interventions on distinct request fields. NexusWeave runs the original request, each replacement alone, and both replacements together. [NIST design of experiments glossary](https://www.itl.nist.gov/div898/handbook/pri/section7/pri7.htm)

The interaction is the change in the recorded-decision rate when both texts are replaced minus the sum of the two changes measured separately. It is expressed in percentage points. A failed trial in any condition or a baseline that never reproduces the recorded decision leaves the interaction unavailable.

The [local model pair example](../examples/local-mlx-pair/evidence.json) uses the same store policy and FAQ in two synthetic conversations. With greedy decoding, the model gave the same answer in all three trials per condition.

| Question | Original | Policy withheld | FAQ withheld | Both withheld | Interaction |
| --- | --- | --- | --- | --- | --- |
| Use the current store policy | 30 days | 60 days | 30 days | 60 days | 0 pp |
| Use the policy or the FAQ | 30 days | 30 days | 30 days | 60 days | −100 pp |

```sh
cargo run --locked -- serve \
  --replay examples/local-mlx-pair/report-policy-first.json \
  --replay examples/local-mlx-pair/report-either-source.json \
  examples/local-mlx-pair/evidence.json
```

Open **Patterns → Replay effects** and expand the pair to compare the calls. The joint change was −100 points in both conversations. The interaction shows where the two sources could substitute for each other.

### Compare measured effects across conversations

Pass one `--replay` per report before the evidence files. In **Timeline**, expand **Tested inputs and effects** on a call to see each individual or joint test and open its trial outcomes.

On a finding card, each input shows its own replay effects. A joint effect appears when the replay changed both inputs in that exact finding for the same decision. The recurring findings view counts findings with a usable individual or joint result in the selected scope. Open a finding to see the intervention, match rule, trial counts, and any available interval.

In **Patterns → Replay effects**, select a thread, a user within an organization, an organization, or all imported runs. The breakdown counts each recorded decision once within each comparison. A decision shows **Mixed results** when its measured replays disagree in direction, including a measured change alongside a measured zero. **Effect unavailable** means every replay for that decision lacked a usable measurement. Select a result to filter the evidence table and open any row to see its exact intervention and trials. The table orders records by the original call timestamp where available.

The comparison groups require the same source and version when supplied, captured text or structured value, context kind and role, operation and replacement, runner ID, model, decision kind and label, match rule, and sampling design. A pair comparison requires both context changes to match. Each report retains its own baseline and variant counts. The displayed ranges describe the measured changes across those tests. Supply different runner IDs for different runner or model configurations.

```sh
cargo run --locked -- inspect --replay data/return-replay.json fixtures/support.json
```

The JSON output includes `replay_effects` with per-test counts, original call timestamps when available, and run, call, decision, and replay IDs. Loading an identical report twice counts it once.

### Recorded local-model example

The MLX replay report records a synthetic return-policy task run on LFM2.5-1.2B-Instruct MLX 8-bit with greedy decoding. The model chose 30 days in all three baseline trials. It chose 60 days in all three trials with the policy withheld and 30 days in all three trials with an unrelated store name withheld. The negative control comparison reports a 100 percentage point drop in the recorded decision rate for the policy edit relative to the store name edit. The comparison uses the saved trials and carries no statistical interval for these greedy runs. [MLX replay report](../examples/local-mlx/report.json)

```sh
cargo run --locked -- serve --replay examples/local-mlx/report.json examples/local-mlx/evidence.json
```

To rerun on Apple Silicon, install `mlx-lm==0.32.2` in a Python environment and substitute the path to the local model directory. The runner loads from that directory with Hugging Face downloads disabled.

```sh
mkdir -p data
cargo run --locked -- replay examples/local-mlx/evidence.json examples/local-mlx/case.json data/local-mlx-replay.json -- python3 scripts/replay_mlx_window.py /path/to/local-model
```

### Inspect activations on the return-window decision

Load the measured activation report with the local-model example. In **Conversation**, select the answer and expand **Activation probe** if it is collapsed. In **Decisions**, use **View activation probe**. Select a decoder block to see the patched log odds and token positions. The edit buttons open the original policy and store-name messages.

```sh
cargo run --locked -- serve \
  --activation-report examples/local-mlx/activation-report.json \
  --replay examples/local-mlx/report.json \
  examples/local-mlx/evidence.json
```

The Python runner generates the report separately on Apple Silicon with `mlx-lm==0.32.2` and the pinned model downloaded locally. The Rust inspector reads the result without loading a model. `inspect` accepts the same `--activation-report` flag and includes matching results in `activation_reports`. [Experiment and measured results](inference-attribution-plan.md#first-instrumented-result), [Pinned model revision](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit/tree/ce5150fd0de58d2bba8d88cc87f13e4fbda5977a)

```sh
python3 scripts/probe_mlx_return_window.py /path/to/local-model \
  --model-id LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit \
  --revision ce5150fd0de58d2bba8d88cc87f13e4fbda5977a > activation-report.json
```

### Follow context influence across an answer

This example measures how edits to two policies and an unrelated store name affect the return-window and refund-time tokens. In **Activation probe**, the first table compares source swaps across both checkpoints for the selected decoder block. Select a checkpoint to see the fixed earlier answer, the original and edited log odds, and the measurements across all blocks. Expand **Generated answers from the original and edited prompts** to see where the greedy answers changed. [Experiment and measurements](inference-attribution-plan.md#follow-two-predictions-in-one-answer)

```sh
cargo run --locked -- serve \
  --activation-report examples/local-mlx-timeline/activation-report.json \
  examples/local-mlx-timeline/evidence.json
```

To reproduce the saved report, use the same pinned model and Python environment as the single-position probe.

```sh
python3 scripts/probe_mlx_answer_timeline.py /path/to/local-model \
  --model-id LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit \
  --revision ce5150fd0de58d2bba8d88cc87f13e4fbda5977a > answer-timeline.json
```

### Find repeated activation effects across conversations

Load the six-conversation synthetic cohort. Open **Patterns → Activation effects**, then choose a thread, user within an organization, organization, or all imported runs. Select a decoder block to compare ranges and counts. Select a decision count to see the original and patched scores for each occurrence, then open one to inspect that exact checkpoint and block on its recorded decision.

```sh
cargo run --locked -- serve \
  --activation-report examples/local-mlx-timeline/cohort/activation-reports.json \
  examples/local-mlx-timeline/cohort/evidence.json
```

The same flag accepts a single report or an array. Use `inspect` with these options to read `activation_patterns`, including the individual scores and locations. The groups require the same model revision, runtime, tokenizer, decoder-block count, decision kind and label, captured source text and metadata, edited token range relative to the source, replacement, and checkpoint contrast. At each checkpoint the group also requires the exact earlier generated tokens. A formatting difference can split one checkpoint while leaving an earlier checkpoint comparable.

Open **Date range** to include calls from a set of UTC dates. **Through** includes the whole selected day. **Compare date ranges** shows two periods at the same scope, thread, and decoder block. Each column links to its contributing decisions, and a decision can appear in both if the ranges overlap. Calls without a captured start appear under **All dates**. Copy the view link to retain the ranges and block. The cohort uses synthetic call dates. To try a comparison, select October and November 2026 under `demo-retail-north / customer-01`.

The view counts each recorded decision once per row. A lower score or preference flip counts only when all comparable measurements of that decision agree. The range includes all loaded measurements for that row. These counts describe experiments that were loaded and matched. [Cohort results](inference-attribution-plan.md#compare-the-measured-pattern-across-conversations)

### Inspect a tool choice and its argument

The routing example records a proposed stock lookup. The first checkpoint compares `stock` with `price`. The second compares the two product IDs while holding the earlier `stock` proposal fixed. Open **Activation probe** on the tool proposal, then expand **Tool proposals from the original and edited prompts** to see the parsed calls from free generation. The experiment stops before execution. [Routing experiment and results](inference-attribution-plan.md#trace-influence-through-a-tool-proposal)

```sh
cargo run --locked -- serve \
  --activation-report examples/local-mlx-routing/activation-report.json \
  examples/local-mlx-routing/evidence.json
```

### Emit activation measurements from another runtime

Write a [version 2 timeline report](../schema/activation-timeline-2.json) and load it with `--activation-report` in `inspect`, `serve`, or `mcp-stdio`. Use the same run, call, decision, and context IDs in the [evidence record](../schema/evidence-1.json). Hash each source's saved UTF-8 text with SHA-256. For a text decision, record the exact generated text in `baseline_answer`. For a tool call, retain the original text and supply its parsed name and arguments in `tool_calls.baseline`. The inspector compares those fields to the recorded decision. The adapter is responsible for parsing its runtime's output.

For each checkpoint, include the zero-based output token index, the fixed earlier text, and the two token IDs in original-then-alternative order. Report the difference between their log probabilities before and after each decoder-block swap. Use matching prompt token indices in both runs and exclusive ends for token ranges. Include a clean-to-clean and a full-donor check at the reported block. The inspector checks report structure, source text, outcome agreement, alignment metadata, and the recorded controls when it loads the report. It can read saved measurements without the model or tokenizer.

### Inspect the same task across runtimes

The saved example uses the same messages and edits with Liquid AI's LFM model in MLX and Qwen2.5-0.5B-Instruct in Transformers with PyTorch on CPU. Both emit the same report format. In **Patterns → Activation effects**, the model and runtime appear on each row. Select a decision count to open its exact checkpoint. The two models use different generated prefixes and decoder-block counts. The view keeps their measurements separate. [Experiment and measured results](inference-attribution-plan.md#test-the-report-across-two-runtimes)

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/activation-report-mlx.json \
  --activation-report examples/runtime-comparison/activation-report-transformers.json \
  examples/runtime-comparison/evidence.json
```

The CPU adapter uses `torch==2.11.0` and `transformers==4.57.6` in a Python environment. Download the pinned Qwen weights to a local directory before running it. The adapter reads local safetensors and supports Qwen2 text generation. [Pinned Qwen model](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/tree/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2)

```sh
python3 scripts/probe_transformers_answer_timeline.py /path/to/local-qwen \
  --model-id Qwen/Qwen2.5-0.5B-Instruct \
  --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 > qwen-activation.json
```

For the matching MLX run, use the [pinned MLX model](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit/tree/ce5150fd0de58d2bba8d88cc87f13e4fbda5977a) and pass `--case examples/runtime-comparison/case-mlx.json` to the existing MLX runner.

### Inspect every recorded output token

The full-output report measures the log probability of each recorded token after a source swap. In **Activation probe**, expand **All 22 output tokens** on the tool proposal. Select a token or use its column heading to inspect the original, edited-prompt, and patched scores. The block picker applies to the full-output view and the selected contrasts. **Copy link** opens the same token and block on the recorded decision. [Experiment and measurements](inference-attribution-plan.md#measure-the-entire-recorded-output)

```sh
cargo run --locked -- serve \
  --activation-report examples/local-mlx-routing/output-token-report.json \
  --activation-report examples/runtime-comparison/output-token-report-transformers.json \
  examples/local-mlx-routing/evidence.json examples/runtime-comparison/evidence.json
```

Add `--all-output-tokens` to either Python timeline runner to include the optional `output_scores` object. One sweep records the effect of swapping a source at each decoder block across the output sequence. The original earlier output tokens remain fixed in every run. Negative changes reduce the probability of the particular recorded token. To measure only output tokens, leave `checkpoints` empty in the case and keep the source edits. If you supply contrasts, the report includes their separate, two-candidate log-odds measurements.

An MCP client can page through the same scores with [`get_output_token_effects`](../schema/output-token-effects-request-1.json). Use the report ID from `get_turn_evidence` or an occurrence in `query_activation_effects`. Pass the captured decision's `location`, and select a zero-based `block` and token `offset`. Each row includes a deep link. Token strings and IDs require the host's `--include-excerpts` flag and the request's `include_excerpts: true`.

### Continue decoding after a source swap

Open **Decoding after a source swap** in the activation probe. Select the source, decoder block, and starting token from the measured interventions. The view shows the fixed earlier output and three continuations. The clean swap reproduces the recorded tokens. The source swap uses the original prompt, and the prompt edit supplies the comparison condition. **Copy link** opens the same intervention. [Experiment and results](inference-attribution-plan.md#let-the-model-continue-after-the-swap)

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/generation-branches-transformers.json \
  examples/runtime-comparison/evidence.json
```

To reproduce the report, add `--all-output-tokens --decode-branches` to the Transformers command above. Declare each intervention in the case's `generation_branches` array using `source_context_id`, a zero-based `layer`, and a zero-based `start_output_token`. The CPU adapter re-runs the prompt and the held output prefix with the source swap in place at every decoding step. Each continuation uses greedy decoding and records whether it reached the stop token or its token limit.

MCP clients can use [`get_generation_branches`](../schema/generation-branches-request-1.json) with a recorded decision location and the report ID in `get_turn_evidence`. The result includes first changes and inspector links. Generated excerpts require the host's `--include-excerpts` setting and the request's `include_excerpts: true`.

### Test source visibility on a recorded answer

Open the saved Qwen measurement in **Conversation** or **Decisions**, then choose **Activation probe → Context visibility test**. **Across the answer** shows where each source changed recorded token scores. Select a token column to rank sources at that position, or use **Previous tokens** and **Next tokens** for longer answers. Open a source to see its recorded prompt fragment. The selected-token list shows why any source could not be measured.

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/source-visibility-transformers.json \
  examples/runtime-comparison/evidence.json
```

The adapter uses the captured request and the existing source anchors. It supports greedy Qwen2 generation and UTF-8 ranges in `/messages/<index>/content`. It verifies that unmasked generation reproduces the recorded decision, then holds earlier output tokens fixed during scoring. The versioned report records coverage for every context item, including those without an exact token alignment. [Report schema](../schema/source-visibility-1.json), [Experimental results](inference-attribution-plan.md#screen-captured-sources-by-masking-attention)

To reproduce the saved measurement with Transformers 4.57.6 and PyTorch 2.11.0, point the script at the local pinned Qwen2.5-0.5B-Instruct checkpoint.

```sh
python3 scripts/probe_source_visibility.py /path/to/local-qwen examples/runtime-comparison/evidence.json \
  --model-id Qwen/Qwen2.5-0.5B-Instruct --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 \
  --run-id transformers-return-and-refund --call-id model-inference --decision-id return-and-refund \
  > source-visibility.json
```

### Test source visibility on a tool proposal

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/visibility-routing/report.json \
  examples/runtime-comparison/visibility-routing/evidence.json
```

In **Conversation** or **Decisions**, open **Activation probe → Context visibility test**. Choose **Single-source effects** and select `stock` to compare the tool definition and request type. Select either digit in `42` and choose **Find routing.product_id** to open Patterns for the full argument. A named target uses the sum of token log probability changes across its span.

The adapter renders the recorded tool list with the request, then requires the generated tool name and arguments to match the captured decision. It parses the extra outer brace pair emitted by this Qwen2.5 checkpoint and scores the original output tokens. The probe only generates and scores the proposal.

Tool definitions align when the captured JSON matches exactly one object in the recorded tool list. The report includes that object's JSON Pointer. The adapter decodes each proposed mask span to verify its text. It accepts extra ASCII whitespace at the edges when the source shares a token with that whitespace. The report records the exact additions under `boundary_whitespace`. In this fixture all seven sources align, including the system instruction with two trailing newlines and each tool definition with one. [Routing experiment](inference-attribution-plan.md#measure-a-proposed-tool-name-and-argument)

```sh
python3 scripts/probe_source_visibility.py /path/to/local-qwen examples/runtime-comparison/visibility-routing/evidence.json \
  --model-id Qwen/Qwen2.5-0.5B-Instruct --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 \
  --run-id qwen-product-route --call-id model-inference --decision-id product-route \
  --target routing.tool 7 8 --target routing.product_id 17 19 \
  --pair request-type product-id --pair tool-stock tool-price \
  --pair routing-instruction request-type --pair routing-instruction tool-stock > routing-visibility.json
```

### Test a pair of sources

Use `--pair FIRST SECOND` with the context IDs from the recorded call. Repeat the option for other pairs. `--pair-duplicates` also tests sources with identical captured text at separate prompt positions, with one additional forward pass for each distinct pair. Both sources need an individual measurement and disjoint token ranges.

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/paired-visibility-transformers.json \
  examples/runtime-comparison/paired-visibility-evidence.json
```

Open **Activation probe → Context visibility test** and choose **Pair interactions**. The matrix shows where the effect of masking both sources differs from the sum of the individual effects. At token 14, both refund-policy copies together sharply lower the probability of the recorded `4`. Select the refund policy and store name to see the control at the same token. Switch the measure to see which sources matter individually. **Find recurring effects** carries the selected measure and token into Patterns. [Paired experiment and measurements](inference-attribution-plan.md#test-whether-two-sources-back-each-other-up)

```sh
python3 scripts/probe_source_visibility.py /path/to/local-qwen examples/runtime-comparison/paired-visibility-evidence.json \
  --model-id Qwen/Qwen2.5-0.5B-Instruct --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 \
  --run-id qwen-redundant-refund-policy --call-id model-inference --decision-id return-and-refund \
  --pair-duplicates --pair refund-policy store-name > paired-visibility.json
```

### Continue decoding with sources masked

Load the saved routing continuations to compare what the model generated with each selected mask. The probe keeps the earlier output tokens and prompt positions fixed, then lets the model pick every remaining token. An all-visible run from each selected start must reproduce the recorded output. The adapter checks the mask at every decoder block and generation step. Tool proposals are recorded as text and are never executed. [Experiment results](inference-attribution-plan.md#let-the-model-decode-with-sources-masked)

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/visibility-routing/continuations.json \
  examples/runtime-comparison/visibility-routing/evidence.json
```

In **Conversation**, a turn with saved results shows **Saved masked replays**. Open its evidence to see each source or pair, its starting token, and what changed. Select **Open replay** to compare the recorded and masked outputs side by side, then **Back to conversation** to return to the same turn. In **Patterns**, a contributing turn links to its continuation when the mask and starting token match the measured source and output span.

The MCP App opens these measurements from `get_turn_evidence`. Under **Masked replays**, open a saved report to read its results. The same turn evidence is available as structured data.

Use `view.kind` of `continuations` with `query_source_visibility` or `POST /api/source-visibility`. Optionally filter by `output_token` and `context_ids`. Results give the first changed token, stopping condition, and whether the parsed tool name or arguments changed. Generated text requires `include_excerpts: true` and the MCP host's `--include-excerpts` flag. [Query schema](../schema/source-visibility-query-1.json)

```sh
python3 scripts/probe_source_visibility.py /path/to/local-qwen examples/runtime-comparison/visibility-routing/evidence.json \
  --model-id Qwen/Qwen2.5-0.5B-Instruct --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2 \
  --run-id qwen-product-route --call-id model-inference --decision-id product-route \
  --target routing.tool 7 8 --target routing.product_id 17 19 \
  --pair request-type product-id --pair tool-stock tool-price \
  --pair routing-instruction request-type --pair routing-instruction tool-stock \
  --continue-mask 7 tool-stock --continue-mask 7 tool-stock tool-price \
  --continue-mask 17 product-id > routing-continuations.json
```

### Query masked-source effects from an agent

`get_turn_evidence` and `/api/turn-evidence` list `source_visibility_reports` for the selected decision, including the number of captured sources, measured sources, and measured pairs. Use its location and report ID with [`query_source_visibility`](../schema/source-visibility-query-1.json) or `POST /api/source-visibility`. The default timeline returns the strongest measured source and pair at each recorded output token. The `sources` view ranks by the absolute change in log probability and retains unavailable entries. Measured rows include the JSON Pointer or extra boundary whitespace when recorded. The `pairs` view ranks by absolute interaction and returns both individual scores and the joint score. Pass `next_offset` as `offset` with the same view to continue.

For example, use this request with the saved paired report. API token indices start at zero.

```json
{
  "version": 1,
  "location": {
    "run_id": "qwen-redundant-refund-policy",
    "call_id": "model-inference",
    "decision_id": "return-and-refund"
  },
  "report_id": "qwen2-source-visibility-8d890be0a43319cc",
  "view": { "kind": "pairs", "output_token": 13 }
}
```

Open a returned `inspector_path` to select the measured token and pair, or open a `source_inspector_path` to see the recorded prompt fragment. **Copy link** in the probe preserves its current token and pair. Generated token text and IDs require `include_excerpts: true`, and for MCP they also require the host's `--include-excerpts` flag.

### Find recurring masked-source effects

Load the four-conversation [synthetic cohort](../examples/runtime-comparison/visibility-cohort/evidence.json) and its [measured reports](../examples/runtime-comparison/visibility-cohort/reports.json). Use the files with the inspector or `mcp-stdio`.

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/visibility-cohort/reports.json \
  examples/runtime-comparison/visibility-cohort/evidence.json
```

Pass the same request to [`query_visibility_patterns`](../schema/visibility-pattern-query-1.json) or the local HTTP route. The example finds pairs containing the copied refund policy at the recorded refund digit.

```sh
curl -fsS http://127.0.0.1:8040/api/visibility-patterns \
  -H 'Content-Type: application/json' \
  --data '{"version":1,"kind":"pair","output_token":13,"source_id":"refund-policy-copy","min_conversations":2}'
```

In the browser, open **Patterns → Masked-source effects**. Choose **Pair interactions** at token 14, then select `refund-policy-copy` to find the same group. Open it to see its contributing turns in order and follow their token and source links. From a visibility report, **Find recurring effects** opens this view at the selected token and uses the captured user or organization scope when available. **Copy view link** preserves the filters and open group.

The default `source` kind reports single-mask changes in log probability. The `pair` kind reports joint interactions in log units. Use `scope` to select a user or organization, `thread` for an exact conversation identity, or `after` and `before` for an inclusive-start, exclusive-end RFC 3339 time window. Copy a group `key` into `group` to page through chronological occurrences and open their exact token, pair, or captured source links.

Use **Order by** in the browser or MCP App to switch between **Most conversations** and **Largest measured effect**. The default puts distinct conversation count first and breaks ties by the largest absolute effect. The effect order reverses those priorities. It uses the most extreme measured occurrence in each group and can surface a single outlier. For HTTP or MCP, pass `"order":"magnitude"` to use it. Combine it with `min_conversations` to require repeated evidence.

Choose **Compare with** to place two scopes side by side, or select the same scope twice and set different UTC date ranges. In HTTP or MCP, pass the second selection as `compare` with its own `scope`, `thread`, `after`, and `before`. Groups align on their comparison key, and `min_conversations` keeps a group when either side reaches the threshold. A side with no matching measurement is `null`. When opening a group, use `offset` and `compare_offset` to page through the two scopes independently.

In a comparison, each group's rank uses the higher conversation count and the largest absolute effect across the two scopes.

Exact-token groups require the same model revision, runtime, tokenizer, decision label, emitted token prefix through the scored token, and source identities. Each identity uses the declared source ID, version, kind, role, captured text digest, masked token count, and any recorded extra boundary whitespace. Older reports without boundary metadata stay readable. A mask with declared boundary whitespace forms its own group. Captures without declared source IDs are counted as unavailable for grouping. The summaries count distinct identified conversations, users, and organizations, alongside decisions and measurements. The cohort's dates and ownership labels are synthetic. Across its four measured decisions the refund policy pair's interaction ranges from −5.84 to −5.18 log units at the recorded refund digit. The range describes those measured decisions.

### Follow a named output across different answers

A workflow can add a `targets` entry with a stable key and recorded token range to a [source visibility report](../schema/source-visibility-1.json). For example, `"targets": [{"key":"returns.refund_duration","token_range":[13,15]}]` labels the two tokens for `4 days` in one answer. Use zero-based start and exclusive end positions. Give the same field the same workflow-qualified key in other answers, even when its position or wording changes.

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/visibility-cohort/reports.json \
  --activation-report examples/runtime-comparison/visibility-target/report.json \
  examples/runtime-comparison/visibility-cohort/evidence.json \
  examples/runtime-comparison/visibility-target/evidence.json
```

In **Patterns → Masked-source effects**, select **Pair interactions** and set **Align by** to `returns.refund_duration`. You can also open the named target from a measured token in the turn's activation probe. For HTTP and MCP, pass `"target":"returns.refund_duration"` in place of `output_token`. Select `refund-policy-copy` as the source to see the same pair measured in two north conversations and one south conversation. The two answers place `4 days` at tokens 14–15 and 8–9. The joint interaction summed over the span ranges from −5.88 to −4.19 log probability units. The target is declared in three of the five reports. [Alternate measured answer](../examples/runtime-comparison/visibility-target/report.json)

Named targets group by key, model revision, runtime, tokenizer, and source identity. The score sums the measured token changes over the declared span. Each token keeps its own recorded earlier output, so the range describes measurements from different prefixes. Drill into the group for each span's positions, prefix digest, and inspector link to its first token. The workflow supplies what the target means.

### Compare measured tool routes

The synthetic routing cohort contains seven measured proposals and 28 saved masked continuations across six conversations, four scoped users, and two organizations. One north conversation has two measured turns. Its second request includes the prior user request and assistant reply in the model input. The saved reports open without model weights. [Cohort inputs](../examples/runtime-comparison/visibility-routing-cohort/cohort.json)

```sh
cargo run --locked -- serve \
  --activation-report examples/runtime-comparison/visibility-routing-cohort/reports.json \
  examples/runtime-comparison/visibility-routing-cohort/evidence.json
```

In **Patterns → Masked-source effects**, set **Align by** to `routing.tool` and **Source** to `stock`. Set **Recorded tool** to `stock`, then set **Compare with** to **All imported runs** and **Recorded tool (B)** to `price`. Open the `stock` definition group to see the measured turns on both sides. Use the organization, user, and thread filters or the UTC timeline to narrow the same comparison. Each contributing turn shows which tool the model proposed.

Read **Replayed** under each scope or UTC period to see how many decisions had a saved continuation for that mask and output span. The stock-definition comparison shows three tool changes on the `stock` side and four matching proposals on the `price` side. Open a contributing turn's **Open masked continuation** link to read its output. For HTTP and MCP, the same counts are in `continuation_outcomes.decisions_tested` and `continuation_outcomes.by_outcome`, with `decision_count` giving the denominator.

Each decision contributes one outcome per group. Multiple continuations with the same category count once. If the saved outcomes differ, the group counts that decision as mixed. Tool changes take precedence over argument changes within one continuation. Token limits and unparsed proposals have their own categories. Only observed categories appear in `by_outcome`.

Apply the same comparison through the HTTP endpoint or the MCP tool. The `tool_name` filter matches the recorded proposal on its side and works with both single-source effects and pair interactions. A named output lets the same source group include different proposed names. [Query schema](../schema/visibility-pattern-query-1.json)

```json
{
  "version": 1,
  "target": "routing.tool",
  "source_id": "stock",
  "tool_name": "stock",
  "compare": {"tool_name": "price"}
}
```

To reproduce both evidence and measurements, use the same pinned Qwen checkpoint and environment as the single proposal example. The script requires every proposal and output span to match its declared case. [Experiment results](inference-attribution-plan.md#compare-routing-effects-across-conversations)

```sh
python3 scripts/probe_routing_visibility_cohort.py /path/to/local-qwen /tmp/routing-cohort
```

### View a measured pattern over time

Open a group under **Masked-source effects** and choose a day, week, or month in **Group by UTC**. The rows show when the selected pattern was measured, its effect range, and its contributing conversation and decision counts. The range markers share a scale centered on zero. With **Compare with** selected, the two scopes use the same calendar rows, and a scope with no measurements in a period has an empty range. Open a row to read the turns for that period. **Copy view link** preserves the grouping and selected period.

Through HTTP or MCP, include the group's `group` key and `"timeline":{"unit":"week"}` in the original query. Weeks start on Monday, and dates come from the captured call start in UTC. The timeline returns populated periods in order. It lists measurements with unavailable timestamps under `undated` when the date filters include them. Add the returned row's `period` to `timeline` to retrieve its occurrences. The recurring threshold applies to the group before the period is selected, so a week with a single measured conversation remains visible.

## Development

See the [contributor guide](../CONTRIBUTING.md) for local checks and [credits](../CREDITS.md) for sources and dataset licensing.
