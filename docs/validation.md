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

The layered dashboard tests also verify the difference between distinct conversations and requests without a conversation ID. In the saved demo, 11 identified conversations have findings and five have a measured change. Browser checks open the four-arm joint-edit result, verify the counts for each selected arm, and follow a scoped occurrence back to its original span. These are scripted journeys. Ease of use with workflow engineers still needs a user study.

The comparison test attaches two different wordings to one live decision. A different runner or scoring rule cannot generate the comparison. Retrying the same report keeps one recommendation. When a repeat under identical wording disagrees with its earlier results, the sidecar withdraws the example that depended on it. The browser opens both original replay links and checks the source-specific answer text beside each count. A separate [local replication with reversed source order](methods.md#check-the-result-against-new-parcel-codes-and-source-order) retains all 28 generated answers.

The source-order check sends the four replication requests through OTLP, attaches eight replay reports and finds four request pairs. Two pairs show different single-edit counts. Filtering to a user who saw just one order removes all pairs. Adding a conflicting repeat removes only the affected pair. Rebuilding the SQLite projections from version three restores the same counts and report links. The browser opens a linked trial and verifies the selected source.

## What the public traces support

The pinned Exgentic retail sample contains eight conversations and 99 model calls. On the unannotated import, the sidecar found 101 exact repetitions across 83 calls in all eight conversations. Every repetition was dialogue from unnamed assistant or user messages. The normalized calls had no declared rules, bound experiment reports or replay-ready request bodies. The traces support locating those repetitions. A causal change in the historical model's answer remains unmeasured.

This result exposed a problem with recommending removal whenever two texts match. The factor list now labels these cases **Repeated dialogue** and places them after named input repetitions. Its recommendation asks whether a workflow inserted a copy. Genuine conversation history may repeat naturally. To reproduce, import the pinned sample, then run the optional check from the repository root.

```sh
cargo test --locked public_retail_recommendations -- --ignored
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

## Public multi-document QA

Ten public questions were each run with ten passages in two orders. The scorer chose a passage for testing before decoding. Its suggested edit changed the generated text in 8/10 cases in the first order and 10/10 in the second. Testing the first passage changed 8/10 and 6/10 respectively. Rotating the inputs changed the top-ranked source on all ten questions. The replay loader verifies the original and edited requests through the Rust planner and scorer. CI recomputes the saved rankings, counts and order comparison without loading a model. [Design, timings, reproducible findings and dashboard](../examples/public-qa/README.md)
