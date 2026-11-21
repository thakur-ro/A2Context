# The revised sidecar missed a tool contract failure on a new agent

A fresh deployment produced **zero recommendations** on both LangChain’s SQL agent and LlamaIndex’s citation engine. The two workflows made 25 successful provider calls. Four of five SQL sessions then stopped at tool validation. All five citation sessions returned text to the user. We ran the ten questions once using the detector revision and stopping rule published before connecting the workflows. [Frozen protocol](protocol.md) · [All ten sessions, including failures and exact call references](outcomes.json)

| Frozen feed | LangChain SQL agent | LlamaIndex citation engine |
| --- | ---: | ---: |
| Sessions | 5 | 5 |
| Model calls with captured inputs and outputs | 20 | 5 |
| Application exits with an error | 4 | 0 |
| Distinct findings | 0 | 0 |
| First five recommendation slots filled | 0 | 0 |

Time to a useful recommendation was unobserved. Precision among displayed recommendations is undefined for both workflows. The tool-reference score was 100% across five eligible SQL traces at 100% coverage. Every tool result present in those requests had a corresponding earlier invocation. A terminal proposal rejected before a result arrived falls outside what this score measures. No retrieval trace was eligible for the score.

## What the quiet review found

Codex reviewed the six sessions selected by trace-ID hash. Each had a concrete issue worth investigating. Codex also develops A2Context. The review establishes source-checked misses in this sample. An application maintainer has not confirmed their relative priority.

| Session | Exact model call | Finding and next action |
| --- | --- | --- |
| SQL 1 | `90ac410598e64648` | The model proposed `sql_db_query` with a `schema` argument. Its tool definition required `query`. LangChain rejected the proposal and the application exited. Check the proposal against the advertised tool schema before dispatch. |
| SQL 4 | `6eb3b0d84af241c1` | The answer reported seven days with changes. The workflow called a query checker and received SQL text. The captured history contains no database query returning seven. A read-only count on the supplied database returned three. Verify the tool result before presenting a number. |
| SQL 2 | `83ff69bbb1ea42ec` | The final proposal supplied `schema` and `table_names` for the query tool. The required `query` field was absent. The application reported a validation error for that field. The same failure also ended sessions 3 and 5 outside the sample. |
| Citations 5 | `bdb1a3c4417745aa` | The answer supplied an arXiv link ending in `2021.12345` under “Source 2.” That URL does not occur in the captured request. Compare the displayed link with the retrieved source before publishing it. |
| Citations 1 | `3766d67f57064594` | Asked about context attribution, the reply discussed water becoming wet when the sky is red. That topic appeared in the engine’s worked example and nowhere in the retrieved sources or real question. Check whether the example is influencing answer content. [Controlled edit](example-edit.md) |
| Citations 2 | `97245d90a5904845` | The answer reported a 48.7 MiB peak and omitted the requested resource limits. A retrieved passage explicitly described a run limited to one CPU and 512 MiB. Check how the answer connects a measurement to its particular test conditions. |

The worked example is a useful comparison with PaperQA. In one workflow, the model copied a formatting identifier. In the other, its answer repeated an irrelevant topic from a demonstration. A detector restricted to opaque citation keys missed the second case. The raw tool proposals exposed a different opportunity: compare the proposed arguments with the actual schema in the same request, then distinguish a predicted validation failure from an observed application rejection.

## What the next detector caught

A fresh sidecar running the subsequent revision displayed one recommendation across four SQL conversations. It surfaced after the fourth exported span. Before dispatch, the same check identified all four proposals that LangChain’s own argument validator rejected. The other ten proposals passed the framework validator and the sidecar reported no missing field. Three of the four rejected proposals supplied a field declared on a different tool in the same request. The dashboard links the required field, the actual proposal and that competing declaration when present. The retrieval feed remained empty. This second pass used the development captures that informed the detector. [All 14 comparisons](proposal-checks.jsonl)

We tested two actions an engineer could take. Telling the model the missing field produced more schema-valid proposals than a generic error. Every resulting query still failed to compile. Renaming a field on the neighboring tool produced no measured transfer of that name. [Repair experiment](repair.md) · [Neighboring tool experiment](sibling-field.md)

To replay both workflows into an empty current sidecar, run `python3 examples/ordinary-usage/replay_feed.py data/ordinary-usage/transfer-frozen --workflows sql citations --port 18474`. To check each proposal with the independently shipped validator, use the pilot’s Python environment to run `python examples/ordinary-usage/transfer/check_proposals.py data/ordinary-usage/transfer-frozen/sql --port 18474`.

## Reproduce

The pilot used Python 3.13, a read-only SQLite connection, the engine’s BM25 retriever, a local LFM2.5-1.2B-Instruct model and the documents from the [original pilot](../protocol.md). The workflow environment used the following direct dependencies.

```sh
uv venv --python 3.13 /tmp/a2-transfer
uv pip install --python /tmp/a2-transfer/bin/python \
  langchain-community==0.4.2 langchain-openai==1.8.5 langchain-classic==1.0.8 \
  llama-index-core==0.14.26 llama-index-llms-openai-like==0.9.0 llama-index-retrievers-bm25==0.9.0
python3 examples/ordinary-usage/transfer/pilot.py data/transfer-run \
  --python /tmp/a2-transfer/bin/python --binary /path/to/frozen/a2context --model-port 18742
python3 examples/ordinary-usage/transfer/record.py data/transfer-run
```

The capture adapter only forwards requests to the loopback model server. The separately saved application logs establish the actual exits. Other providers, model sizes and domains remain untested by this pilot.

With the original capture directory present, `python3 examples/ordinary-usage/transfer/verify.py data/ordinary-usage/transfer-frozen` resolves all six review references and verifies the missing query fields, application rejections and independent database count.
