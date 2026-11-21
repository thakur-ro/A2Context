# Study recommendations across real agent runs

Can an engineer find a recurring context problem, locate the decision it precedes, and choose a useful test when the workload contains thousands of conversations?

## Data and study design

| Cohort | Use | Evidence available |
| --- | --- | --- |
| Exgentic Agent LLM Traces v2, revision `ccd4c607cb708b4bb9e043bdc63a9013df35a9bc` | Recommendation quality and dashboard journeys across six benchmark workloads | Captured benchmark-agent requests and responses, tool definitions, harness, and run-level outcome |
| FreeInference Agentic Trace | Receiver and dashboard load from human-driven sessions | 12,002 sessions and about 1.19 million request records, with timing, token counts, tool events and input mutations. Message text was withheld. |
| Instrumented benchmark runs with controlled changes | Measure whether changing a source changes a tool choice, answer or verified state | Named source edits, original-request controls, model and harness identity, and independent outcome checks |

Exgentic covers AppWorld personal assistance, BrowseCompPlus research, SWE-bench software work, and τ²-bench retail, airline, and telecom support. These are model executions in benchmark environments. The published variant contains inference calls. It preserves the messages carried into each request, including available tool calls and results. Saved telemetry supports statements about what was captured and what preceded a result. Controlled interventions support narrower claims about whether a specific change affected the measured outcome. [Pinned dataset card](https://huggingface.co/datasets/Exgentic/agent-llm-traces-v2/blob/ccd4c607cb708b4bb9e043bdc63a9013df35a9bc/README.md) · [FreeInference dataset](https://huggingface.co/datasets/harvardMadsys/freeinference_agentic_trace)

Process the complete Exgentic release and report actual session, request, and capture counts by workflow. Keep published run outcome as metadata. Freeze a task-level split before defining any recommendations that use task outcomes. Group runs from the same underlying task across seeds, models, and harnesses when the upstream IDs permit that grouping. Reserve at least one workflow for transfer, and report unknown task identity separately. Record shard hashes, binary revision, import counters, and detector settings for every pass.

## Measure usefulness

1. Freeze the current detector output as a baseline. Compare its top five per workflow with simple rankings by frequency and recency. Stratify inspection by workflow, success status, conversation length, and evidence coverage. Audit both surfaced recommendations and missed problems in sampled conversations.
2. Give reviewers the captured sources, provenance, and a short rubric. Score whether the card names a specific problem, links to exact evidence, distinguishes missing capture from measured behavior, and offers a next step an engineer could take. Record borderline cases and disagreements. A failed benchmark episode is an outcome label and does not by itself identify a context cause.
3. Check the high-ranking findings and missed problems. Change one detector, grouping rule, or interaction per iteration. Re-run the same development set and retain the previous result. Run the frozen holdout once the change is specified. Report top-five actionability, problem coverage in the reviewed sample, and counts by evidence type. Ground causal language in a matched replay or other controlled intervention.
4. Exercise the full dashboard with the imported database. Find a recurring problem, inspect what changed between calls, and open affected conversations and exact tool evidence. Measure time to the first usable recommendation and time to exact evidence. Check pagination, filter persistence, and mobile layout. Record latency distributions with concurrent reads and ingestion, along with storage, peak RSS, rejections, queue drops, and missing content. Repeat with a million captured request records for the operational load target.

The first pass reuses recorded runs and makes zero paid API calls. Budget a separate, capped set of controlled interventions after the observed examples are reviewed. Retain the results and exact reproducer commands with the study.

## Import the public corpus

Use the nine Parquet shards under `data/train` in the [pinned dataset](https://huggingface.co/datasets/Exgentic/agent-llm-traces-v2/tree/ccd4c607cb708b4bb9e043bdc63a9013df35a9bc/data/train). Store local data under the ignored `data/scale/` directory. Build the sidecar and install PyArrow in your Python environment.

```sh
cargo build --release --locked
target/release/nexusweave sidecar --listen 127.0.0.1:18052 \
  --db data/scale/study.sqlite --capture-content \
  --metadata benchmark.harness --metadata benchmark.success \
  --metadata benchmark.original_trace_id
```

From another terminal:

```sh
python scripts/import_exgentic_scale.py \
  --sidecar http://127.0.0.1:18052 --content \
  --report data/scale/exgentic-import.json data/scale/exgentic-v2/*.parquet
```

The adapter maps each inference span to a deterministic trace ID so long sessions fit the current per-trace storage limit. The original trace ID and inference span ID identify the source call, and the original session ID reunites the calls in Threads. The import retains source timestamps. Individual requests larger than the export budget retain metadata and are counted in the report. Parent spans and separate tool spans are outside the scope of this dataset variant. A second import of the same row IDs targets the same stored calls.

## What the first pass found

The baseline receiver ingested all nine pinned Exgentic shards. Its database contained 10,056 distinct sessions and 241,473 inference calls with captured inputs. It held no declared rules or replayable request bodies. The database occupied about 80 GiB on the test Mac. The full content import took about 12 minutes on a release build, including a retry of earlier trace IDs. This run measures the current storage path and exceeds the original sidecar storage expectations for a small deployment.

| Workflow | Sessions | Captured calls |
| --- | ---: | ---: |
| AppWorld personal assistance | 1,500 | 48,453 |
| BrowseCompPlus research | 1,948 | 38,405 |
| SWE-bench software work | 1,959 | 91,768 |
| τ²-bench airline | 957 | 10,916 |
| τ²-bench retail | 1,848 | 21,947 |
| τ²-bench telecom | 1,844 | 29,984 |

Two of the top five cards on the first ten AppWorld sessions pointed to the literal `(no content)` marker that accompanied assistant tool calls. Some copies ended in newlines. Excluding these marker parts from duplicate scoring reduced the recommendation count from 407 to 127 on the same 241 requests. The marker stays in captured evidence. A plain assistant message containing the same text remains eligible for duplicate scoring.

An intermediate pass took the first 100 recorded sessions from each workload in shard order. Desktop and mobile runs opened exact evidence in all six workflows. All 30 of the top five cards per workflow were repeats of dialogue without named sources. The result identifies a limit of the existing detectors on this corpus. An exact repeat can warrant a closer look, and its effect on any output requires a separate test. The sample favors early shards, which group related harnesses and model runs.

Use the audit script after ingestion finishes. It resolves each top card to two exact inputs and compares the ranked list with frequency and recency. Its automatic labels describe the evidence behind a card. Human review is required for usefulness labels.

```sh
python scripts/audit_scale_recommendations.py --sidecar http://127.0.0.1:18052 \
  --db data/scale/study.sqlite --report data/scale/recommendation-audit.json \
  appworld browsecompplus swebench tau2_airline tau2_retail tau2_telecom

node scripts/check_scale_browser.mjs http://127.0.0.1:18052 \
  appworld browsecompplus swebench tau2_airline tau2_retail tau2_telecom
```

## Run the metadata load study

Download the `processed/requests.parquet` and `processed/sessions.parquet` files for each week from the [pinned FreeInference release](https://huggingface.co/datasets/harvardMadsys/freeinference_agentic_trace/tree/bb26cf72c9c20f2b05f765d88548156a74d3c253/release). Rename them `WEEK-requests.parquet` and `WEEK-sessions.parquet` where `WEEK` is the upstream week directory. Use a separate database to keep its capture policy and results independent of the content study.

```sh
target/release/nexusweave sidecar --listen 127.0.0.1:18055 \
  --db data/scale/freeinference-load.sqlite \
  --metadata study.input_transition --metadata study.step_trigger
```

```sh
python scripts/import_freeinference_scale.py \
  --sidecar http://127.0.0.1:18055 \
  --report data/scale/freeinference-import.json data/scale/freeinference/*-requests.parquet
```

The adapter joins the request rows to anonymized accounts and harness names. The harness becomes the workflow filter, and the reconstructed session becomes the conversation. It retains stable identities within the source weeks. The release includes an evaluation harness alongside interactive harnesses. Counts from this study report gateway request records and do not measure prompt quality. Cite Nixon et al. when using the CC BY 4.0 dataset. [From Requests to Sessions](https://doi.org/10.5281/zenodo.23903440)

## Full-corpus dashboard check

The updated build passed 12 exact-evidence journeys, one on desktop and one on mobile per workload. On this Mac the opening recommendation view took 0.82–4.34 seconds across the 12 checks. Evidence drilldown took 0.12–0.15 seconds and retained the workflow and call in the URL. The updated coverage query took 2–8 ms in these runs. An earlier baseline AppWorld desktop run took about 60 seconds under concurrent disk reads, and its mobile run exceeded the 60-second wait. Cache state and competing work differed between the runs. Treat the timings as observations from this machine rather than a controlled speedup estimate.

The opening page now requests a compact summary instead of calculating the detailed impact view up front. When all groups in the scope are exact repeats, it leads with a repeat review and the exact examples. A recurrence view chooses distinct conversations on the server, so one long conversation cannot use every example slot. [Recommendation improvement loop](recommendation-loop.md)

## Million-call operational run

The receiver imported the processed request and session tables for all 16 weeks of the pinned FreeInference release. The stored totals were 1,222,199 inference calls, 14,036 reconstructed session keys, 267 anonymized accounts and 14 harness names. The processed tables flatten nested sessions, so these counts include rows outside the dataset card's top-level session and request totals. The receiver reported zero rejected batches and queue drops during the bulk import. Reopening the database with the updated binary preserved all 1,222,199 calls. The resulting database occupied about 4 GiB on the test Mac.

The earlier build's unfiltered Threads query took a median of about 13 seconds over five local reads on a 1.03-million-call snapshot. It read trace rows to build the list. The updated query reads thread keys and timestamps from an index for unfiltered browsing. **Threads** is now in workspace navigation.

On the full database, desktop and mobile each opened the unfiltered list, applied a workflow filter, paged forward, opened a session, and used a link that focused the exact call. The unfiltered list opened in 2.85 and 3.33 seconds. Applying the `codex` workflow filter took 125 and 116 ms, opening the next page took 102 and 114 ms, and opening a session took 118 and 99 ms. Those are two runs on one machine. They measure browser journeys on recorded metadata. Throughput budgets for a live inference deployment require a separate workload test.

```sh
node scripts/check_large_threads.mjs http://127.0.0.1:18055 codex
```
