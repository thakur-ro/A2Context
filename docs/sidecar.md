# Run NexusWeave beside inference

The sidecar receives [OTLP/HTTP](https://opentelemetry.io/docs/specs/otlp/) trace exports and writes normalized evidence to embedded SQLite. The dashboard opens on **Recommendations**. It counts distinct identified conversations with findings, and groups them by what the captured evidence shows. Select a count to open the conversations and the measurement behind each one. Select a source factor to see where its findings recur. Each occurrence links to its original conversation and span. URL filters carry across those layers.

```sh
cargo build --release --locked
target/release/nexusweave sidecar --db ./evidence.sqlite --metadata deployment.environment.name
```

Use `http://127.0.0.1:8040/v1/traces` as the traces endpoint of an asynchronous OTLP exporter. The sidecar accepts OTLP JSON or Protobuf, optionally gzip encoded. It acknowledges success after the SQLite transaction commits. An exact retry preserves a single copy per trace and span ID. A late span merges into the existing trace, including a parent span that arrived after its child. Trace exports for unsupported instrumentation can be stored as topology until a supported span arrives.

Existing detail links resolve through the sidecar when it has the referenced trace. Legacy thread or workspace snapshots are limited to eight traces or 8 MiB of trace data. The conversation and request lists support pagination.

### Read recommendations from another dashboard

The following routes return JSON. Pass the same `org`, `user`, `workflow`, UTC dates (`after` inclusive, `before` exclusive) and allowlisted metadata pair (`meta_key`, `meta_value`) through each step. Omit dates to include undated observations. A paged response contains `rows` and `next_offset`. Supply that offset on the same query to continue.

| GET route under `/api/sidecar/` | Use |
| --- | --- |
| `impact` | Get distinct identified conversations, counts by evidence category (`evidence_counts`), requests without conversation IDs and source factors. |
| `conversation-outcomes?level=1` | Page through conversations in one category. Levels 1 to 5 correspond to `proposal`, `rate`, `score`, `tested` and `inputs`. Omit the level for all categories. Each row identifies the finding, trace, call and measured comparison. |
| `factors` | List versioned source sets, the strongest signal associated with each set (including differing results across comparable replacement texts), and the counts in scope. Select the opaque `id` of a factor. |
| `order-tests?factor=…` | Compare single-edit match counts across paired source orders for a factor. Both requests must pass the current filters. Each row links to both original reports and includes the total number of matched comparisons and how many differed. |
| `replay-variations?factor=…` | Compare single-source edits across distinct captured inputs. Each row gives counts, one changed and one equal example, their exact replay locations and whether only fragment order differs between those two requests. |
| `findings?factor=…` | Get the recommendations for that factor. Each includes an action, comparison details and an example occurrence. Use `group` to select a recommendation by ID. |
| `occurrences?factor=…&group=…` | Page through the individual observations behind the recommendation. Each row carries its own measurement, request and call IDs, and conversation key when available. |
| `token-path` | Read up to 16 measured output positions for a token finding. Pass its `group`, `trace`, `call`, `decision`, `report`, and zero-based decoder `block`. Use `next_offset` to read the next page of `tokens`. Source and generated excerpts require demo mode or content capture. |

The dashboard counts each identified conversation once after applying the current filters. A conversation can move to another category when new findings arrive or a filter excludes a measurement. Within a category, the most recent eligible finding supplies the evidence link. Requests without a conversation ID contribute to a separate count. [Category definitions](methods.md#what-changed-in-a-conversation)

Expand **Scored replays** on the overview or a source factor to see how many conversations had changed match rates, equal rates, both results across comparisons, or no scored replay. The four counts in `impact.replay_outcomes` add up to the identified conversations with findings in that scope. Probe scores contribute to the broader measured-change count. Only replay comparisons contribute to this breakdown. Two equal match rates can come from as little as one trial per arm. Open the recommendation to check the trial counts and scoring rule.

### See what happened when two sources changed together

Open the Cedar dispatch recommendation. The cards show the original answer, an edit to each source, and the joint edit. Under **Where did this pattern recur?**, select the result to open either of the two conversations. The counts separate reports, identified conversations and distinct captured JSON requests. Each report compares the same edits, model, runner, rule, and number of trials per condition. A conversation can contain different results. [Open Cedar in the demo](http://127.0.0.1:8040/?recommendation=602208c5e9c068fab608137a9c92699923591b1f349ab8504b57e65084f85808) · [Compare the wordings and scoring rules](methods.md#when-both-single-edits-preserve-the-answer)

![The Cedar replay shows the original answer and each edited answer, with two conversations behind the joint-only result.](media/joint-proposals.png)

For clients, a joint replay card returns `joint_patterns` for the selected factor and trial count. Request `occurrences` with its `group`, `factor`, `joint_pattern` and `baseline_trials` to page through the supporting reports under the same filters. Each occurrence identifies the trace, call, decision and replay. A report without captured request JSON contributes to `missing_inputs`. Distinct input counts preserve array order and ignore JSON object key order.

### Follow a source edit through an answer

Open a token recommendation. It selects the output position and source edit with the largest measured shift. Choose another token to see which tested edits affected its probability. Expand a source to see the replacement and the answer the model generated with it. The coverage count lists captured inputs that still need a test. [Open the refund example](http://127.0.0.1:8040/?recommendation=6158b8bd5e64d129e9be699f6c78c0965402edc0b7367df671e02ff744171bb1#output-token-14)

![The refund-policy edit changes the probability of the original four-day window and the separately generated answer says seven days.](media/token-path.png)

In the tool example, the model wrote `42` in a stock lookup and repeated it in the explanation. Choose **Compare the same token at position 21** to see how the effect of editing the product ID differs across the two positions. The edited generation shows which argument the model actually proposed. [Open the tool example](http://127.0.0.1:8040/?recommendation=a40da3144994322a6a46f98bd239219307918b750e9626c0f2247b19c8597a95#output-token-8)

### Find an edit whose effect varies across inputs

**Edit effects varied across inputs** opens two examples where the same edit produced different match-rate results. The sidecar groups the source identity, version and text, operation and replacement, model, runner, decision kind and label, match rule, sampling design and trial counts within one source factor. Each distinct captured JSON request counts once. Object keys are sorted for this count and arrays keep their order. An input with both results across repeats is counted separately. The recommendation requires at least one consistently changed input and one consistently equal input. Unscored edits supply neither example.

For named fragments in a string field, the sidecar checks the captured UTF-8 bytes and excludes overlapping or ambiguous inventories from the order comparison. It fingerprints the surrounding request, the gaps between fragments, and a sorted inventory of each eligible field's source identities, versions and contents. A matching fingerprint and different positions of the edited source establish that the displayed requests differ only in fragment order. Several sources can move together. Sampling can also affect replay results. Each example links to its own trial. With more than two eligible inputs, the view uses a deterministic example from each group. [Public QA comparison](../examples/public-qa/README.md#compare-an-edit-across-inputs)

### Measure an edit before the first output token

Open **Before inference** and choose **Try a measured tool choice**, then **Check the same edits before the first token**. In the saved Qwen example the request-type edit moved the first-token distribution by 0.000169 bits. The store-name edit moved it by 0.00000469 bits. Follow **Compare the measured tool proposals** to inspect the separate greedy replays. The request-type edit switched the proposed tool from `stock` to `price`. The store-name edit kept `stock`. Both used product ID 42. [Saved prefill measurement](../examples/runtime-comparison/pending-routing/prefill-preview.json) · [Saved proposals](../examples/runtime-comparison/pending-routing/shadow-report.json)

![The measured tool-routing edit and the relative first-token shifts for both sources.](media/prefill-ranking.png)

To run the same probe with local Qwen weights, supply the existing named edits and an explicit scorer. Each tested edit requires one model forward pass in addition to the shared baseline. The scorer reads pre-sampling logits and generates zero tokens. The output file includes the captured request.

```sh
routing=examples/runtime-comparison/pending-routing
jq '.interventions' "$routing/case.json" > /tmp/routing-edits.json
nexusweave probe-prefill "$routing/request.json" /tmp/routing-edits.json /tmp/routing-preview.json -- \
  python3 scripts/score_qwen_prefill.py /path/to/qwen/snapshot --model-id Qwen/Qwen2.5-0.5B-Instruct \
  --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2
```

Load the output file in **Before inference**, or pass `{"request": <file contents>}` to `POST /api/sidecar/preview` or `inspect-pending`. MCP clients can pass the same envelope as the `input` to the pending evidence query. The returned `prefill.ranked` entries carry a source identifier, version, request pointer, edited request digest and divergence in bits. The sidecar verifies those bindings without calling the model. Verification relies on the adapter for its reported logits-derived measurements and checkpoint identity.

Other model adapters can implement the same batch interface. On stdin, version 1 supplies `baseline`, its `baseline_request_sha256`, and 1–32 single `edits` with exact request bodies, source IDs, versions and digests. The adapter returns version 1, model ID and revision, runtime, tokenizer hash, baseline digest and one effect per edit in input order. Each effect echoes the context ID, source ID, version and request digest, and adds `js_bits` in [0, 1]. Declare byte ranges when equal text occurs in multiple fields. Full-vocabulary access is required by this metric. [Reference adapter](../scripts/score_qwen_prefill.py) · [Algorithm and interpretation](methods.md#which-source-would-you-test-without-candidate-answers)

### Follow a first-token test across conversations

Copy the measured request’s `nexusweave.prefill.probe` attribute to the inference span alongside the original request body and its source declarations. OTLP attributes can carry the probe as a JSON string. LangSmith imports accept the same key in run metadata. A sidecar with content capture enabled verifies the request and edit digests, source versions, and model identity when it indexes the span. With content capture disabled, the report is discarded with the request body.

In **Recommendations**, open **First-token sensitivity**, then review a comparison. The card counts distinct captured request bodies. It keeps ties, all-zero measurements and disagreed repeat rankings separate. Opening a conversation shows the tested edit, its shift in bits and any completed paired proposals from an attached shadow report. One observed change applies to that input and runner.

![First-token ranking on two inputs, with the original and edited routing proposals for one conversation.](media/prefill-conversations.png)

To reproduce the routing example, send the four saved measurements to a sidecar with `--capture-content`. Select organization `retail-north` for the repeated result or `retail-south` to inspect the stock-only case.

```sh
python3 examples/runtime-comparison/pending-routing/prefill_cohort.py --endpoint http://127.0.0.1:8040/v1/traces
```

Agents can query `findings?kind=prefill` with the usual organization, workflow, user and date filters. Each row returns `prefill_counts` for ranked sources and `prefill_followup` for outcomes grouped by runner. A follow-up requires a consistent unique leader and one completed baseline and edited proposal in a verified shadow report for that source and exact edited request. The comparison uses the full proposal kind, label and outcome. The UI shortens long values. A missing pair contributes no outcome. Repeated paired checks that disagree contribute one mixed input.

Each group holds the tested source identities, versions, original values, edit operations, replacements, locations, model revision, tokenizer, runtime and non-user request settings fixed. Plain user text may vary in `messages` and `input`; tool inventories and system instructions stay in the comparison key. Unnamed sources stay within their run. A retained raw span can recover a report omitted by an older importer during the archive upgrade. Unverifiable reports leave a reason on the span and produce no ranked finding. The MCP `get_turn_evidence` tool returns the verified ranking for the selected call. Source text and replacement excerpts require both host permission and an explicit request. [Measured examples and reproduction](methods.md#did-the-leading-edit-change-the-tool-call)

### Test instruction text and tool definitions

The routing comparison tests whether changing an instruction or tool inventory changes a user-source edit’s effect. Open the saved example from **Before inference → Try a measured tool choice → Test instruction text and available tools**. A live recommendation surfaces the same table when the captured span carries the experiment.

To reproduce the eight proposals with the local Qwen checkpoint and Python environment used by the first-token adapter, start from a fresh output directory:

```sh
routing=examples/runtime-comparison/pending-routing
python3 "$routing/prefill_cohort.py" --prepare /tmp/nw-routing-study
nexusweave test-routing /tmp/nw-routing-study/routing-south-stock-only-scored.json \
  "$routing/routing-design.json" qwen2.5-0.5b-instruct-eaa56b5-transformers-cpu-greedy \
  /tmp/nw-routing-study/repeated.json -- python3 scripts/propose_qwen.py \
  "$MODEL_DIR" --model-id Qwen/Qwen2.5-0.5B-Instruct
```

Load the output in **Before inference**, or attach its `nexusweave.prefill.probe` attribute to the corresponding inference span. On your own scored chat request, choose a verified edit within a user message, the index of a system or developer message, its alternate text, and the alternate array of tool definitions. The CLI passes each full request to the explicit proposal runner on stdin. The runner returns a JSON choice with `kind`, `label` and `outcome`. Use a proposal-only runner, since dispatching calls would change the experiment.

The sidecar reconstructs all eight requests and checks their digests before comparing complete choices. A failed trial leaves its comparison incomplete. The producer supplies the runner identity and trial outputs. The eight trials belong to one example and add zero conversations to the dashboard totals. MCP returns the pattern and trial statuses by default. Reading instruction excerpts, tool names or returned arguments requires excerpt access.

### Compare a routing test across conversations

Select **Repeats disagreed** on the overview to compare four runs on one input. The **Tool changed** column counts switches between proposed tools or between text and a tool call. Under the original instruction with both tools available, two runs switched tools. A third changed only the text answer. Expand **Compare tool selection** to open the exact setup in a changed and an unchanged run. [Sampled example](http://127.0.0.1:8040/?org=routing-sampling) · [Experiment and reproduction](../examples/runtime-comparison/pending-routing/sampled.md)

![Repeat counts for full proposals and tool selection under four instruction and inventory combinations](media/repeat-trials.png)

Open the **Routing tests on 4 inputs** factor in the built-in demo. The table groups inputs by their four original-versus-edited outcomes. The first column of outcomes compares the complete choice. The second compares the proposed tool name. Two text answers both have no selected tool. When two tools share a name the comparison cannot distinguish them by name alone. Open **Inspect example** to see the exact proposals and expand **Other user messages** to read the surrounding text.

For an instrumented workflow, query `/api/sidecar/findings?kind=prefill` with the usual scope filters. Each `routing_profiles` row counts distinct request digests and links to one trace and call. The four positions in `signature` and `tool_signature` correspond to original settings, alternate tools only, alternate instruction only and both alternates. A `1` denotes a change, `0` an unchanged result and `?` an incomplete pair. The sidecar assigns `mixed` to a metric only when two completed pairs gave opposite results for the same input and setup. Otherwise it uses the most complete single report, with a stable trace and call tie-breaker. Complementary partial reports stay partial. For the input behind each example, `repeat_counts` shows changes and missing pairs per setup and `example_runs` supplies the denominator. When repeats disagree, `repeat_evidence` links to a changed and an unchanged run for the first disputed setup of each metric. The overview opens that comparison directly. Coverage counts distinct measured inputs and identified conversations separately. A missing routing report adds no vote.

The comparison holds the parent finding’s source edits and model settings fixed. It also matches the selected source edit, instruction slot, alternate instruction text, both full tool inventories and runner ID. The card selects a design with completed trials, preferring more fully measured inputs. For another captured design, follow its occurrence link or filter by `routing_design`. Distinct conversation IDs can refer to identical input bodies. A repeated body contributes one input to its design.

To export the saved holdout to a running sidecar with content capture:

```sh
python3 examples/runtime-comparison/pending-routing/replicate_routing.py | \
  curl --fail-with-body -H 'Content-Type: application/json' --data-binary @- http://127.0.0.1:8040/v1/traces
```

To repeat the model measurements, pass `--model "$MODEL_DIR" --output /tmp/routing-holdout-results.json` to the same script with the Qwen adapter environment. [Protocol and results](methods.md#does-the-tool-inventory-result-repeat-on-fresh-requests)

### Run the position test on a matched comparison

**Download position test** prepares eight conditions. On each of the two captured backgrounds, it keeps the other named fragments in their relative order, puts the selected source at each tested position, and applies the original replacement at each slot. The download contains the full request bodies. Preparation makes zero model calls. **Attach completed trials** binds each returned proposal to its background, replay, source, slot, replacement state and request digest.

On the overview, a factor with contrasting edit effects shows the latest position-test counts when both requests match the active filters. Within the replay tier, a test that changed both proposals with the source present and neither with it replaced comes first. This ordering rule describes two backgrounds on the submitted runner. Accurate answers and repeatability on new requests require separate checks.

Agents can read `position_test` on a `factors` row to get the group, pair, trial counts and latest result. It is null when there is no receipt for the current comparison. If a factor has several tested comparisons, the overview prefers the two-background result described above, then the most recently submitted receipt. The detail view lists the other comparisons.

Agents can follow the same route. From a `replay-variations` row with a `position_pair`, carry the current scope plus `factor`, `group`, `pair=position_pair`, `baseline_trials` and `edit_trials` to `GET /api/sidecar/position-plan`. Read the two trial counts from the changed example's baseline and variant measurements. A stale pair or a filter that excludes one background makes the plan unavailable. Each condition references a compact JSON string in `requests` by SHA-256. Numbered slots are one-based. Each background retains its original position and evidence location.

```sh
nexusweave run-position-plan plan.json my-runner-revision trials.json -- my-proposal-runner
```

The explicit runner receives one JSON request on stdin and returns a proposal with `kind`, `label` and `outcome` on stdout. The CLI checks the table and hashes before invoking it once per condition. A failed or unreadable proposal is recorded as failed. The plan names the historical runner. The CLI argument names the runner you actually use. A tool-call experiment should return the proposed call for comparison.

Submit `{"selection":{…},"result":{…}}` to `POST /api/position-results` using the same scope and selection fields. The sidecar reconstructs the plan and verifies the trial bindings, then compares the full returned proposals across slots separately for the intact and replaced source. Each count requires two completed proposals on the same background. The model outcomes come from the runner. An identical submission, including a reordered trial list, returns the existing receipt. The data expire with either underlying trace. Up to 4 MiB of position results can reference a trace. The card shows the latest submitted result. `GET /api/sidecar/position-results` accepts the selection and returns receipt summaries with `report_offset` pagination. Add `id` for one complete set of eight outcomes. [Public example and saved trials](../examples/public-qa/README.md#run-the-downloaded-test)

### Preview an assembled request

Choose **Check a new request against earlier edits** in the demo. Earlier replays suggest testing the dispatch guide. Choose **Compare with model scores for this request** to see why editing the routing note reversed the model's preference between `UNKNOWN` and `KITE` on the pending Ash input. Both views use saved measurements. The dashboard makes zero model calls. [Runnable scored request](../examples/redundant-routing/candidate-checks/ash-routing-note.json) · [Held-out results](methods.md#what-do-model-scores-add-before-generation)

For live traffic, supply `nexusweave.organization.id` and the same user identity as the captured calls. The producer defaults to `otlp`. For another producer, set `nexusweave.history.source` to the stored run source. A missing user matches only calls with no user ID. Named, versioned sources must bind to verified text fragments or complete message content. Settings, tools, message roles, fixed text and source positions must agree. Only the text inside bound sources and a separate final plain user question can vary.

The preview's `history` field returns up to five tested edits from the latest 512 eligible observations for this layout and identity. Each edit needs two distinct earlier input bodies after excluding the pending body. Fully scored single edits with a fully matching baseline qualify. Lower, equal and mixed counts each count a request once. Mixed means repeats disagreed about whether the match rate fell. The response flags a truncated window. Missing scope or anchors leave the current request's own checks available.

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
