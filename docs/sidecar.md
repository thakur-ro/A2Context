# Run NexusWeave beside inference

The sidecar receives [OTLP/HTTP](https://opentelemetry.io/docs/specs/otlp/) trace exports and writes normalized evidence to embedded SQLite. The dashboard opens on **Recommendations**. It counts distinct identified conversations with findings, including the subset with a measured score or outcome change. Choose a source factor and a recommendation to see the tested conditions. Each occurrence links to its original conversation and span. URL filters carry across those layers.

```sh
cargo build --release --locked
target/release/nexusweave sidecar --db ./evidence.sqlite --metadata deployment.environment.name
```

Use `http://127.0.0.1:8040/v1/traces` as the traces endpoint of an asynchronous OTLP exporter. The sidecar accepts OTLP JSON or Protobuf, optionally gzip encoded. It acknowledges success after the SQLite transaction commits. An exact retry preserves a single copy per trace and span ID. A late span merges into the existing trace, including a parent span that arrived after its child. Trace exports for unsupported instrumentation can be stored as topology until a supported span arrives.

Existing detail links resolve through the sidecar when it has the referenced trace. Legacy thread or workspace snapshots are limited to eight traces or 8 MiB of trace data. The conversation and request lists support pagination.

### Read recommendations from another dashboard

All four routes return JSON. Pass the same `org`, `user`, `workflow`, UTC dates (`after` inclusive, `before` exclusive) and allowlisted metadata pair (`meta_key`, `meta_value`) through each step. Omit dates to include undated observations. A paged response contains `rows` and `next_offset`. Supply that offset on the same query to continue.

| GET route under `/api/sidecar/` | Use |
| --- | --- |
| `impact` | Read the number of distinct identified conversations with findings, the subset with a measured change, the number of requests without conversation IDs, and the number of source factors. |
| `factors` | List versioned source sets, the strongest signal associated with each set (including differing results across comparable replacement texts), and the counts in scope. Select the opaque `id` of a factor. |
| `findings?factor=…` | Get the recommendations for that factor. Each includes an action, comparison details and an example occurrence. Use `group` to select a recommendation by ID. |
| `occurrences?factor=…&group=…` | Page through the individual observations behind the recommendation. Each row carries its own measurement, request and call IDs, and conversation key when available. |

### Preview an assembled request

Open **Before inference** and load a preflight request or a request bundled with previously measured probe reports. The workflow can also call `POST /api/sidecar/preview` with a JSON object containing `request` and optional `candidate` or `shadow` reports. The response includes the evidence and suggested next step. This route uses the bounded worker, processes the request in memory, and makes zero model calls. Use it as a separate check when the workflow needs a result before dispatching the original request. [Preflight format](guide.md#check-context-before-inference) · [Pending inspection format](../schema/pending-inspection-1.json)

To carry the same measurements into the background feed, attach verified reports as `nexusweave.candidate_probes` or `nexusweave.shadow_probes` on the matching inference span. The sidecar checks request identity, source edits, and score or trial arithmetic. [Candidate probes](guide.md#measure-candidate-preference-before-a-decision)

### Capture inputs and sources

The default policy stores a trace skeleton, scope identifiers, model and operation attributes, and explicitly allowlisted scalar metadata. Content capture requires `--capture-content` and a new database if the existing database used a different policy. Redact secrets, prompt data, tool arguments, and personal data in the exporter or collector before using this flag. Only allowlist low-sensitivity keys such as the deployment environment. Scope and service IDs are stored even with content capture disabled, so use pseudonymous IDs where needed. [OpenTelemetry GenAI attributes](https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/)

For input findings, emit standard captured GenAI or OpenInference content attributes and [NexusWeave source declarations](guide.md#declare-context-sources-and-decision-scope). To create repeat groups across threads, attach stable source IDs and versions, and carry the same organization and user keys across those threads. `service.name` supplies the workflow filter. Multiple distinct metadata values for one key within a trace are treated as ambiguous and omitted from filtering.

For a replay where proposals run in a separate job, use `nexusweave plan-replay <evidence.json> <case.json> <plan.json>` to export exact baseline and edited request bytes. Collect the proposals with your runner and call `nexusweave score-replay <evidence.json> <case.json> <trials.json> <report.json>`. The trial file has a `version` of 1, the exported `plan_id`, and `conditions` with `context_ids`, `request_sha256`, and `trials`. The scorer checks the plan, request digests, and trial counts. Neither command calls a model. [Replay case and results](guide.md#try-a-decision-replay)

Upload a completed report to `POST /api/replay-reports` once the captured baseline trace is accepted. Use `POST /api/activation-reports` for a validated report from an instrumented model. A verified, supported report can generate a recommendation with its measured comparison and next test. Try an existing saved report without running a model:

```sh
target/release/nexusweave sidecar --db ./synthetic.sqlite --capture-content
# In another terminal
python3 examples/send_otlp_replay.py
```

Semantic screening and proposal-only checks run as separate opt-in jobs. The sidecar grants them zero automatic model-call budget. Set screening’s pair cap before invoking the [offline scorer](guide.md#screen-captured-text-for-overlap). Set the number of proposals per arm separately in a [shadow case](architecture.md#test-which-source-changes-an-outcome), use a proposal-only runner, and attach the finished capture. These workloads belong outside the sidecar's CPU and memory allocation.

### Deploy in a shared network namespace

Build the image with `docker build -t nexusweave:private .`. In Kubernetes, run it as an additional container in the same pod as the instrumented application or its local Collector. Configure a writable volume for UID 10001 at `/state`. For a provisional limit of one CPU and 512 MiB, a container spec can use:

```yaml
- name: nexusweave
  image: registry.example/nexusweave:private
  args: ["--db", "/state/evidence.sqlite", "--retain-days", "7"]
  resources:
    limits:
      cpu: "1"
      memory: 512Mi
  volumeMounts:
    - name: nexusweave-state
      mountPath: /state
```

Open the dashboard with `kubectl port-forward pod/WORKFLOW_POD 8040:8040`. For Docker on Linux, `--network host` lets a local exporter reach the sidecar on loopback. The dashboard and ingest API share that loopback boundary. Run one deployment per trust boundary and grant port-forward access only to operators authorized to see its evidence. Mount a persistent volume to keep SQLite across replacements and keep the database and its WAL together.

### Fan out without changing observability platforms

One local OpenTelemetry Collector can send the same received trace stream to the sidecar and to your current backend. This sketch uses independent bounded sending queues. Use a separate redaction pipeline or equivalent filter if capture policy differs between destinations.

```yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: 127.0.0.1:4318
processors:
  batch:
    send_batch_size: 32
    send_batch_max_size: 32
exporters:
  otlphttp/nexusweave:
    traces_endpoint: http://127.0.0.1:8040/v1/traces
    sending_queue:
      queue_size: 128
      num_consumers: 1
  otlphttp/observability:
    traces_endpoint: ${env:OBSERVABILITY_OTLP_TRACES_ENDPOINT}
    sending_queue:
      queue_size: 128
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/nexusweave, otlphttp/observability]
```

For LangSmith, configure its [OTLP endpoint and headers](https://docs.langchain.com/langsmith/trace-with-opentelemetry) on the observability exporter. This fan-out sends traces to both destinations. Use the [LangSmith importer](guide.md#import-langsmith-traces) for saved runs. [Collector queue and retry behavior](https://opentelemetry.io/docs/collector/resiliency/)

### Operate the collector

- `GET /api/sidecar/health` returns current queue depth, cumulative 503 admissions, accepted and rejected OTLP batches, latest processing lag in milliseconds, and retained trace counts and inference coverage for inputs, declared rules, and recorded requests. Counters reset on process restart. The lag includes queue wait, processing, and the response write.
- There are eight waiting jobs and one worker. An export may contain no more than 64 trace IDs and 2 MiB encoded and decoded. Merged traces are capped at 512 spans and 2 MiB. Use exporter batching and retry temporary 503 responses. Configure an exporter request timeout.
- Retention defaults to seven days since a trace last changed. An identical retry does not extend retention. A background purge removes expired evidence and dependent report records. SQLite reuses freed pages. Provision and monitor disk for the ingestion rate and capture level, including WAL.
- This mode does not block or re-run inference. For a check before the workflow calls the model, call [preflight](guide.md#check-context-before-inference) with the assembled request.
