# When a retrieval app cites its own prompt examples

A developer asked how to batch telemetry by tenant. The application displayed three sources. One was the batch processor README. The other two, `34-pl` and `28-pl`, came from worked examples in the chain’s prompt. A2Context raised one recommendation to check those references. The developer could follow it to both requests and the reply, without supplying an expected answer. [Recorded question, source labels and displayed answer](followup-results.json)

## Results from ordinary use

| Run | Sessions | What the feed showed | What review found |
| --- | ---: | --- | --- |
| Frozen LlamaIndex database agent | 5 | No recommendations | All three sampled questions were answerable from the database. The agent declined without using its tools. |
| Frozen LangChain retrieval app | 5 | No recommendations | All three sampled answers cited identifiers from unrelated worked examples. |
| Five fresh questions on the same retrieval app after the change | 5 | One distinct recommendation across four conversations | The app displayed at least one example-only reference on all five questions. The first had no earlier conversation to compare. |

Codex performed the developer review against captured calls, application output and the pinned repository. The two frozen applications used different framework components and a new corpus. Their ten model calls returned HTTP 200 and all ten applications exited successfully. Neither feed reached a first recommendation. Both had zero runs eligible for the tool-result reference score. The prospective follow-up used new questions on a familiar application and one small local model. Its single recommendation was actionable. The target of three useful distinct recommendations among the first five on **each unfamiliar workflow** is still open. [Original protocol](protocol.md) · [Frozen outcomes](outcomes.json) · [Fresh-question protocol](followup.md)

The follow-up feed first displayed the issue on question two, **3.48 seconds** after the first capture listener opened. Developer review confirmed it **20.70 seconds** after that listener opened. This includes local application startup and inference. The first answer also mixed real README paths and example IDs. On question three the application listed only example IDs. Finding four occurrences represents one underlying problem. [Timestamps, both witness calls and all five outputs](followup-results.json)

## What the silent runs contained

| Sampled session | Independent evidence checked |
| --- | --- |
| Database 5 | A read-only query found four commits touching `exporter/otlphttpexporter/go.mod` and three each touching the debug and OTLP exporter module files. |
| Database 1 | The three most frequently changed receiver paths had six, three and three commits. The offered catalog included `list_tables`, `describe_tables` and `load_data`. |
| Database 2 | Commit `75c145f99` touched exporterhelper. Its subject added a code owner for profiling modules. |
| Retrieval 5 | The request included the exporterhelper persistent-queue documentation. The application abstained and cited three example IDs. |
| Retrieval 4 | The request included the OTLP receiver configuration. The displayed sources mixed example IDs with the receiver README path, which appeared three times. |
| Retrieval 2 | The retrieved passage distinguished the batch send trigger from its size limit. The displayed answer abstained and cited example IDs. |

The source-label judgment uses what the application actually printed. It needs no assumption about which intermediate tokens or model weights produced the answer. Source IDs and retrieval paths can also legitimately repeat between requests. Each live recommendation asks the engineer to check the cited source. [Pinned Collector corpus](https://github.com/open-telemetry/opentelemetry-collector/tree/8e266ba207ab3ad93e3606578e3b56a5fbd47adf)

## Did changing the shared text change the reply?

Across 30 development sessions from six workflows, the preregistered comparison selected eight final replies. Each had identifiers found exclusively in shared input lines. We renamed those identifiers together, preserved the varying material, and generated two unchanged replies and one edited reply at each of three seeds.

| Workflow | Selected questions | Question–seed pairs | Both controls cited an original | Edited reply switched to its matching alias |
| --- | ---: | ---: | ---: | ---: |
| PaperQA | 4 | 12 | 12 | 10 |
| Collector retrieval | 4 | 12 | 12 | 12 |

All 72 requests completed. None of the edited replies reproduced an original selected identifier. Two PaperQA edits omitted the matching alias as well. The switches support influence of the edited material on **which reference was emitted** under the tested conditions. The eight questions share templates and one model. Repeated seeds do not give 24 independent workflows. The separate PaperQA-specific detector also found the same problem when its formatting cues were present. [Frozen selection procedure](shared-text.md) · [Trial-level observations](shared-text-results.jsonl)

The passive Rust detector takes the final plain-text chat reply in a trace and looks for citation-like identifiers on a reference line or inside parentheses. It compares the nearest earlier capture in the same workflow, model, message-role sequence and recorded identity and metadata scope. It abstains on an overlapping call or the same conversation. It retains up to three identifiers appearing only in shared whole lines, with UTF-8 offsets in both requests and the output. Tool-capable, multipart or oversized inputs fall outside this check. It groups observations by scope and message positions. Passive capture triggers zero model calls. The optional intervention requires access to a compatible model. [Rust implementation](../../../src/sidecar/shared_identifiers.rs)

## Reproduce

The offline check runs without a model or network access.

```sh
python3 examples/ordinary-usage/collector/check.py
```

For fresh inference, start the loopback model described in the [first pilot](../README.md#reproduce). Use a checkout of the Collector containing the pinned revision and its 101 first-parent commits. The adapter reads it with Git and creates a separate read-only application database. Use a new output directory for each run.

```sh
uv venv --python 3.13 /tmp/a2-collector
uv pip install --python /tmp/a2-collector/bin/python llama-index-core==0.14.26 \
  llama-index-llms-openai-like==0.9.0 llama-index-tools-database==0.7.0 \
  langchain-classic==1.0.8 langchain-community==0.4.2 langchain-core==1.8.4 \
  langchain-openai==1.8.5 langchain-text-splitters==1.1.3 rank-bm25==0.2.2
python3 examples/ordinary-usage/collector/run_collector.py data/ordinary-usage/collector-frozen \
  --checkout /path/to/opentelemetry-collector --python /tmp/a2-collector/bin/python \
  --binary /path/to/29b0801/a2context --model-port 18742
python3 examples/ordinary-usage/collector/run_collector.py data/ordinary-usage/collector-followup \
  --followup --checkout /path/to/opentelemetry-collector --python /tmp/a2-collector/bin/python \
  --binary /path/to/1d92630/a2context --model-port 18742
```

To repeat the intervention, pass the six workflow capture directories to `shared_text.py`. The default writes the candidate evidence to stdout. Add `--model-port 18742` to generate all three arms. Each capture directory contains one `sidecar.sqlite`. The published trial rows include request and reply digests. New model runs may differ. [Intervention runner](shared_text.py)

The Linux release binary processed 1,024 synthetic 4-KiB requests at 128.53 calls per second under a 1-vCPU, 512-MiB limit. Sending an eight-call batch and reading its recommendation took 17.38 ms median and 19.34 ms p95. Peak cgroup memory was 36.54 MiB. There were zero queue drops, retries or OOM kills. This benchmark times the background path after export. Use the [container procedure](../../../docs/validation.md) with `--finding shared_identifier` and an empty database. [Measured counters](visibility-cost.json)
