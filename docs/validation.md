# Private release validation

The evaluation uses synthetic trace labels and saved local and hosted model trials. The labels describe which cards an engineer should investigate. This is a test of the initial design and its failure modes. It makes no estimate of production accuracy.

## Reproduce the checks

Use stable Rust and Python 3.10 or newer.

```sh
cargo build --release --locked
python3 -m venv /tmp/nexusweave-otel
/tmp/nexusweave-otel/bin/pip install opentelemetry-exporter-otlp-proto-http==1.45.2
python3 scripts/check_sidecar.py --sdk-python /tmp/nexusweave-otel/bin/python
```

The script uses a direct JSON OTLP producer and the independent OpenTelemetry Python SDK with Protobuf/gzip. Both producers exercise the conflict, recurrence, and replay journeys. The checks cover filter scope, allowlisted metadata, default redaction on disk, late parent spans, retry deduplication, expiry, and restart recovery. Saved Qwen trials exercise the live report route. The validator reproduced three deltas, including a numeric zero. It accepted a separate incomplete trial and excluded the unavailable delta from measured cards. It rejected a tampered request digest. The external replay planner scored the same saved trials in reordered batch form and reproduced the original report.

For browser checks, start `target/release/nexusweave sidecar --demo`. Install `playwright-core` in a temporary directory and use an installed Chrome or Chromium binary.

```sh
PLAYWRIGHT_MODULE=/path/to/node_modules/playwright-core/index.mjs \
CHROME_BINARY=/path/to/chrome \
node tests/sidecar-browser.mjs
```

The checks cover the native recommendations, the request preview before inference, cross-user recurrence with filter persistence, the original span and replay links, and horizontal layout at widths of 1280 and 390 pixels. Set `FINDINGS_SCREENSHOT` to regenerate the [README preview](media/findings.png).

The preview and live-path tests verify a candidate flip, its smaller comparison edit, the report’s binding to the captured request, and a token-level recommendation from an attached activation report. A stale request hash yields an unverified probe and no measured card. Restarting a retained database from the earlier schema version regenerates the recommendation projections.

The history checks exercise different complete answers that both pass the scoring rule, disagreeing repeats, incomplete evidence, and a new request whose source ID and UTF-8 byte range differ from the earlier trial. They verify that an already-applied edit produces no runnable experiment. A verified trial for different replacement text cannot supply the result for the recommended edit. Desktop and mobile journeys follow the earlier evidence link and open the current proposal pair. [Measured example](methods.md#can-an-earlier-replay-identify-what-to-test-on-the-next-request)

On the seeded demo, 25 warm loopback requests that prepared the Ash edit took 0.395 ms median and 0.500 ms p95. Including its saved prefill and proposal reports took 0.439 ms median and 0.529 ms p95. The release binary and Python client shared one Linux container capped at 1 vCPU and 512 MiB, with a 39.7 MiB memory high-water mark. The timing covers the preview HTTP round trip and client serialization on this small database. It excludes model execution. Reproduce with `python3 scripts/check_sidecar.py --history-cost` in the same container.

The layered dashboard tests also verify the difference between distinct conversations and requests without a conversation ID. The saved demo has 21 identified conversations with findings. Thirteen have a changed generated proposal, two have a score shift as their strongest finding, and six have captured-input findings only. In both score cases the proposal stayed the same in the completed routing trials. The browser checks follow a changed journal-policy answer and an unchanged routing example on desktop and mobile. Browser checks open the four-arm joint-edit result, verify the counts for each selected arm, and follow a scoped occurrence back to its original span. These are scripted journeys. Ease of use with workflow engineers still needs a user study.

The proposal coverage check sends a standalone Ash trial in which both answers were `UNKNOWN` and both missed `KITE`. It checks a matching pair, a changed pair with the same match rate, a variable arm without a consistent pair, scope filters, pagination and redelivery. Reopening a version 15 database recovers the suppressed matching results from retained evidence. The browser opens a changed and a matching comparison from the same sampled conversation, then follows each link back to the scoped list.

Replaying four saved routing requests across 256 conversation IDs put 128 conversations in the matching-only group and 128 in the group with both outcomes. In 20 warm reads, fetching the summary and the first 15 conversations with both examples took 3.21 ms median and 3.28 ms p95 per pair of HTTP requests. The release sidecar and benchmark client shared a 1 vCPU, 512 MiB Linux container. Together they peaked at 55.2 MiB with zero queue drops. This measures dashboard reads after ingestion on replicated data. Reproduce with `python3 examples/runtime-comparison/pending-routing/benchmark_routing.py target/release/nexusweave --proposal-coverage`.

The token journey checks two saved models. On Qwen, the returns policy ranks first on the return window and the refund policy ranks first on the refund window. Two of the five captured inputs lack a tested edit. On MLX, the same product ID token appears twice. The browser follows it across pages, checks an argument-only change, and finds a position where all tested edits gave a measured zero. The SDK test verifies that an upgraded database still connects the finding to the changed generation. [Qwen report](../examples/runtime-comparison/generation-branches-transformers.json) · [MLX report](../examples/local-mlx-routing/output-token-report.json)

The comparison test attaches two different wordings to one live decision. A different runner or scoring rule cannot generate the comparison. Retrying the same report keeps one recommendation. When a repeat under identical wording disagrees with its earlier results, the sidecar withdraws the example that depended on it. The browser opens both original replay links and checks the source-specific answer text beside each count. A separate [local replication with reversed source order](methods.md#check-the-result-against-new-parcel-codes-and-source-order) retains all 28 generated answers.

The source-order check sends the four replication requests through OTLP, attaches eight replay reports and finds four request pairs. Two pairs show different single-edit counts. Filtering to a user who saw just one order removes all pairs. Adding a conflicting repeat removes only the affected pair. Rebuilding the SQLite projections from version three restores the same counts and report links. The browser opens a linked trial and verifies the selected source.

The four-condition regression sends controlled trial records through the live API. It covers both single and joint changes, unchanged outputs, and a condition with mixed answers. It verifies that a constant match rate can coexist with a changed proposal, that different sample sizes stay separate, and that the same conversation can contribute different results. A database upgrade from version 13 rebuilds the saved classifications and input counts. Desktop and mobile browser checks follow the Cedar result to its individual comparisons.

The source-group check gives one conversation 16 contributing source groups and lets 16 conversations share the first group. It checks both pages of results, repeated delivery, the exact evidence link and a restricted time window. A separate replay case verifies that drilling into a source with an ineffective edit cannot move a conversation out of its original evidence category. The desktop and mobile journeys follow the FAQ group to a changed answer and back to the same two conversations.

The edited-source check uses two instrumented reports with three tested sources each. In Qwen, editing either returns policy or refund policy changed the generated answer. In MLX, editing either request type or product ID changed the parsed tool proposal. The store-name edit shifted token scores slightly in both reports and preserved both proposals. Taking only the largest absolute token-score change per report would find two of the four observed proposal-changing edits. The edited-source view finds all four. These six tests measure coverage on the saved fixtures. [Qwen trials](../examples/runtime-comparison/generation-branches-transformers.json) · [MLX trials](../examples/local-mlx-routing/output-token-report.json)

The routing checks verify that a completed comparison credits just the edited source, restore the same links from a version-14 database, and open the changed routing setup on desktop and mobile. In a separate four-arm replay, a single edit changed the proposal while the joint edit restored the original. Only the individual edit qualifies in that conversation. [Routing checks](../tests/sidecar_prefill.py) · [Replay checks](../scripts/check_sidecar.py)

The capture-population check covers a trace crossing midnight, a late finding, redacted input, stale screen digests, and rebuilding the call index from a version 16 database. The live journal check verifies that the captured conversation moves from a classifier suggestion to a measured policy edit when the report arrives. Redelivery preserves the same count. The desktop and mobile tests follow the source pair into the replay planner and open the classifier result before inference.

On 256 synthetic, single-call conversations, 128 had declared conflicts and 128 had no finding. The overview and the first 15 conversations without a finding took 1.662 ms median and 2.022 ms p95 across 20 warm pairs of HTTP reads. All 128 distinct conversations appeared across the pages. The release sidecar and Python client shared one Linux container capped at 1 vCPU and 512 MiB. The cgroup peaked at 27.6 MiB with zero OOM events. Run `python3 scripts/check_sidecar.py --population-cost` after building the release binary to repeat this workload.

## What the public traces support

The pinned Exgentic retail sample contains eight conversations and 99 model calls. On the unannotated import, the sidecar found 101 exact repetitions across 83 calls in all eight conversations. Every repetition was dialogue from unnamed assistant or user messages. The normalized calls had no declared rules, bound experiment reports or replay-ready request bodies. The traces support locating those repetitions. A causal change in the historical model's answer remains unmeasured.

Replaying prefixes of each saved conversation shows when repetition first became visible. Each row uses the first calls in dataset order and the same SQLite queries as the dashboard.

| Calls retained per conversation | Captured calls | Conversations with findings | Conversations with comparable proposals |
| --- | ---: | ---: | ---: |
| First call | 8 | 0/8 | 0/8 |
| First two | 16 | 0/8 | 0/8 |
| First four | 32 | 8/8 | 0/8 |
| Full conversation | 99 | 8/8 | 0/8 |

These eight sessions were selected for linked tool results. Their counts describe this sample. Repeated dialogue alone gives no measurement of its influence on any answer.

This result exposed a problem with recommending removal whenever two texts match. The factor list now labels these cases **Repeated dialogue** and places them after named input repetitions. Its recommendation asks whether a workflow inserted a copy. Genuine conversation history may repeat naturally. To reproduce, import the pinned sample, then run the optional check from the repository root.

```sh
cargo test --locked --bin nexusweave public_retail_recommendations -- --ignored --nocapture
```

[Sample and import instructions](guide.md#import-public-retail-traces) · [Pinned sample manifest](../examples/retail-sample.manifest.json)

## Ranking

The development cases and the held-out cases use different source IDs, policy issues, and threads. Both include recurring actionable conflicts, a deliberately harmless duplicate, an intended precedence case, and an issue without machine-readable rule declarations. The held-out comparison includes saved individual and joint Qwen interventions with a separate replay fixture. The evaluator labels the individual and joint edits as separate experiments. It reports unique actionable labels among the first five cards and coverage across all returned cards. The rules-only baseline orders detected conflicts by recency. Newest first sorts every card by observed time.

| Method | Development precision among first five shown | Development coverage | Held-out precision among first five shown | Held-out coverage |
| --- | ---: | ---: | ---: | ---: |
| Findings order | 2/4 | 2/3 | 4/5 | 4/5 |
| Rules only | 2/3 | 2/3 | 2/3 | 2/5 |
| Newest first | 2/4 | 2/3 | 2/5 | 4/5 |

The rules-only baseline supplied three cards, so its denominator for precision is three. It filled two of five available positions with actionable findings. In the ranking review, the newest-first order surfaced an equal-rate replay ahead of actionable cases. The revised order gives recurring conflicts and changed replay outcomes higher priority and retains the other evidence. The highest-ranked low-priority case in Findings remains a genuine declared conflict with accepted precedence. The undeclared override was missed. The UI now reports the fraction of inference spans with any declared rule. The rule detector needs structured declarations. Optional text screening is evaluated separately. [Evidence and ranking rules](methods.md)

## Inference latency

The comparison reused one warm Qwen2.5-0.5B-Instruct model on an arm64 macOS host with PyTorch 2.11.0 and Transformers 4.57.6. Each arm used a real Python SDK 1.45.2 Protobuf/gzip `BatchSpanProcessor` to an existing loopback sink. The treatment added a second processor exporting to the sidecar. The prompt, model, sampler, maximum of four generated tokens, and existing exporter stayed fixed. Alternating blocks produced 72 observations per arm after warmup. The timed region included generation and span completion. Force-flush ran between blocks. All treatment traces reached the sidecar. [Pinned model revision](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/tree/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2)

| Configuration | Median | p95 |
| --- | ---: | ---: |
| Existing asynchronous exporter | 88.860 ms | 89.468 ms |
| Existing exporter plus sidecar exporter | 89.053 ms | 89.911 ms |

The measured median difference was **+0.193 ms**. This is a short single-host inference workload. It does not establish latency for hosted models, other exporters, or concurrent production workflows.

```sh
# Use an environment with the OTLP SDK, torch==2.11.0 and transformers==4.57.6.
python3 scripts/measure_exporter_latency.py /path/to/pinned/qwen/snapshot \
  --revision eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2
```

## Constrained overload

With the source drill-down enabled, ten warm reads of a category’s source groups followed by its leading group’s conversations took a median of **19.28 ms per pair**. The overview and first factor page took 17.13 ms per pair. The sidecar retained all 2,304 traces after 139 of 192 batches needed a retry. It peaked at **49.0 MiB** under the one-CPU, 512 MiB limit, with zero OOM events. The reads ran after the conflict-only burst finished. These timings measure dashboard access, not inference latency.

With conversation evidence categories enabled, the burst below retained all 2,304 traces and placed all 12 identified conversations in the captured-input category. The server accepted 51 batches on the first attempt and recovered the other 141 on retry. The median across ten pairs of overview reads was 20.86 ms after ingestion. The sidecar peaked at 46.9 MiB with zero OOM events. These measurements cover conflict-only inputs and a warm database. Score and proposal reports carry larger payloads.

Use a fresh disposable container so retries can be compared against counts from an empty database. The burst sends 192 exports of 12 spans from 24 clients. Each span has 4 KiB of additional captured input. The load generator runs in a separate container and shares only the sidecar's network namespace.

```sh
docker build -t nexusweave:private-validation .
sidecar=$(docker run --detach --rm --cpus=1 --memory=512m --memory-swap=512m \
  nexusweave:private-validation --db /state/load.sqlite --capture-content)
docker run --rm -i --network "container:$sidecar" --workdir /work/scripts \
  python:3.13-alpine python - --overload-url http://127.0.0.1:8040 \
  < scripts/check_sidecar.py
docker exec "$sidecar" sh -c 'cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory.peak /sys/fs/cgroup/memory.events'
docker stop "$sidecar"
```

On Docker Desktop 28.3.2 with an arm64 Linux container, the cgroup reported a one-CPU quota and a 512 MiB memory limit. It peaked at **45.0 MiB**, recorded **0.234 CPU-seconds** from container start through recovery, and reported zero OOM events or CPU throttling periods.

| Burst metric | Observed |
| --- | ---: |
| Initial accepted batches | 59 of 192 |
| Initial HTTP 503 responses | 133 |
| p95 successful initial export | 20.08 ms |
| Burst duration | 85 ms |
| Recovery of HTTP 503 responses | 133 of 133 in 266 ms |
| Unique retained traces after retries | 2,304 |

The script retries each 503 sequentially after the burst. This run demonstrates bounded admission and recovery under the stated payload and concurrency. Sustained workload capacity depends on trace size, traffic shape, disk, and exporter retry configuration.

The source-factor dashboard was also tested at commit `cd530a1` under the same container limits and workload. All 141 initially declined batches succeeded on retry. The database retained 2,304 requests. The overview reported 12 distinct conversations and 192 source factors. Ten sequential pairs of HTTP reads (impact summary plus the first factor page) took a median of **7.97 ms** after ingestion finished. Peak cgroup memory across ingestion, retries and reads was **48.7 MiB**, with zero OOM events. This measures a warm database after a burst. Concurrent ingest and dashboard latency under sustained load remain unmeasured.

With source-order matching enabled, the same capped burst retained all 2,304 requests after 136 retries. The ten warm overview and factor reads took a median of **11.74 ms per pair**. We then sent the four source-order requests and the inconsistent repeat from `check_order_tests` to the same sidecar. Three eligible comparisons remained. Peak memory over both workloads was **49.4 MiB**, with zero OOM events. The read timings cover the conflict-heavy burst. A replay-heavy production workload still needs measurement.

The two instrumented demo answers span three token pages. In a Linux container limited to one CPU and 512 MiB, 75 sequential warm HTTP reads took a median of 0.358 ms and a p95 of 0.423 ms, including Python JSON decoding. Peak cgroup memory was 38.8 MiB for the sidecar and the benchmark client together. This fixture measures reading saved reports. It excludes instrumentation, model inference and concurrent ingestion. Reproduce with a release build under the same limits.

```sh
python3 scripts/check_sidecar.py --binary target/release/nexusweave --token-page-cost
```

## Public multi-document QA

Ten public questions were each run with ten passages in two orders. The scorer chose a passage for testing before decoding. Its suggested edit changed the generated text in 8/10 cases in the first order and 10/10 in the second. Testing the first passage changed 8/10 and 6/10 respectively. Rotating the inputs changed the top-ranked source on all ten questions. The replay loader verifies the original and edited requests through the Rust planner and scorer. CI recomputes the saved rankings, counts and order comparison without loading a model. [Design, timings, reproducible findings and dashboard](../examples/public-qa/README.md)


## Routing instruction and tool test

The South stock-only fixture carries a four-setup experiment. The local Qwen adapter produced byte-identical output files on two runs of `test-routing`. The sidecar verification suite covers substitution of a request digest, an incomplete trial, an unlisted tool proposal, archive upgrades from versions 8 and 9, JSON and SDK exports, and MCP excerpt permissions. Desktop and mobile browser checks cover both the unscoped demo and the organization-filtered recommendation. [Reproduction](sidecar.md#test-instruction-text-and-tool-definitions) · [Integration checks](../tests/sidecar_prefill.py)

An offline Linux container with one CPU and a 512 MiB limit ran the prebuilt Rust release tests and the JSON routing integration suite. The cgroup peak for the whole run was 397,635,584 bytes with zero OOM events. The release compiler was killed when building under that same limit. Build the binary separately. This run measures test-suite memory and does not estimate serving latency.

## Routing comparisons across requests

With the four saved routing inputs replicated across 256 identified conversations, 128 conversations had a changed proposal from editing `request-type`. After one warm-up, reading the edited-source ranking and the first 15 supporting conversations took a median of **3.84 ms per pair** and a p95 of **3.93 ms** across 20 pairs. The sidecar and Python client shared one CPU and 512 MiB in an offline Linux container. Combined peak memory was 35.7 MiB, with zero queue drops or OOM events. This measures reads after ingestion. Reproduce with `python3 examples/runtime-comparison/pending-routing/benchmark_routing.py target/release/nexusweave --edited-sources`.

The built-in demo and the OTLP exporter display the four inputs fixed in the [replication protocol](../examples/runtime-comparison/pending-routing/routing-holdout.json). A second full run reproduced the four saved reports, including all twelve first-token forward passes and all 32 completed proposals. The integration suite checks that switching the outcome from full proposal to proposed tool changes the count for product 73, and that an exact input seen in two conversations contributes one input. It also checks incomplete and disagreeing trials, runner isolation, evidence links, scope filters and archive recovery.

We measured a warm, organization-filtered findings query on four distinct requests submitted under 256 trace and conversation IDs. In a Linux container limited to 1 CPU and 512 MiB, the previous binary (`33ffae8`) took 4.83 ms at the median and 9.37 ms at the 95th percentile over 20 timed reads. With routing aggregation the same query took 5.03 ms and 5.24 ms. Each run discarded one warm-up read and reported zero queue drops. These timings cover a loopback HTTP query over one comparison design. More designs, distinct inputs or concurrent exports need separate measurements. [Benchmark script](../examples/runtime-comparison/pending-routing/benchmark_routing.py)

```sh
python3 examples/runtime-comparison/pending-routing/benchmark_routing.py target/release/nexusweave
```

### Repeating the same source edit

The sampled Qwen pilot produced four complete sets of eight proposals on one synthetic request. A second execution reproduced the saved artifact byte for byte. In the original-instruction, two-tool setup, the complete proposal changed in 3/4 paired runs and the tool selection changed in 2/4. Tests removed each of the eight trial arms in turn on repeats of four saved greedy inputs. All 32 omissions preserved the original classification. Separate tests introduced an opposite result and verified both evidence links. Desktop and mobile checks follow the sampled example to the highlighted setup and compare the actual outputs. [Protocol, code and raw measurements](../examples/runtime-comparison/pending-routing/sampled.md)

We also submitted the four sampled records under 256 distinct trace and conversation IDs, retaining one distinct input. Each Linux container had 1 CPU and 512 MiB. After one warm-up, the median over 20 scoped recommendation reads was 5.33 ms on the previous release (`afe5587`) and 8.12 ms with repeat counts. The corresponding source-factor queries took 2.67 and 4.62 ms. Both runs had zero queue drops. This workload measures query cost on a warm database after ingest. [Benchmark script, run with `--sampled`](../examples/runtime-comparison/pending-routing/benchmark_routing.py)
