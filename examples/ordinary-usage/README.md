# First field test of the recommendation feed

The frozen sidecar produced **zero recommendations** on ten development sessions. Both applications returned answers. The release criterion of three useful recommendations among the first five on each workflow remains unmet. We stopped after the five questions per application specified in the [protocol](protocol.md).

| Recorded in November 2026 | SQLite agent | PaperQA |
| --- | ---: | ---: |
| Sessions | 5 | 5 |
| Model requests with captured inputs and outputs | 8 | 20 |
| Sessions with a tool-result message | 1 | 0 |
| Distinct recommendations surfaced | 0 | 0 |
| Quiet sessions sampled for independent review | 3 | 3 |

**What the score missed.** The SQLite agent sometimes told the user it could only query a database when that database contained the requested project history. In four sessions it answered without a tool call. In the remaining session it proposed three tool calls and their results appeared in later requests with matching IDs. The tool-reference score was therefore 100% on one eligible session. On PaperQA there were zero eligible sessions and no numeric score. Neither measurement describes the usefulness of an answer.

**What the retrieval application exposed.** One question asked which measurements support the operating budget. The answer referred to decoder blocks and edit substitutions. The frozen feed had no suggestion for this answer. The reviewer can inspect the retrieved text passed to each model call to distinguish inadequate retrieval from a weak final answer. Empty recommendation lists leave precision undefined and time to the first useful recommendation unobserved.

The project owner volunteered to review the quiet-session sample. These observations are the implementer’s reading of the outputs. Independent dispositions are pending. This pilot uses a small local model, project material and one person’s engineering questions. [Per-request record and application output digests](first-pass.json)

We tested one possible explanation after the pilot. One tool description referred to an unavailable tool. Correcting the description produced fewer first-step tool proposals on the same inputs. [Controlled edit, including unchanged repeats](catalog-edit.md)

## Reproduce

Use the current pilot scripts and build the sidecar binary from the revision pinned in the protocol. Start a loopback MLX server with `lmstudio-community/LFM2.5-1.2B-Instruct-MLX-8bit` loaded as its default model. The original run used a 512-token server default. The capture listener forwards only chat completions to that local port. It records model traffic and application exits. Tool execution inside the SQLite plugin and PaperQA’s local search operations have no execution spans in this setup.

```sh
uv venv --python 3.13 /tmp/a2-tools
uv pip install --python /tmp/a2-tools/bin/python llm==0.38 llm-tools-sqlite==0.1
uv venv --python 3.13 /tmp/a2-papers
uv pip install --python /tmp/a2-papers/bin/python paper-qa==2026.8.12
python3 examples/ordinary-usage/run.py tools /tmp/a2-tools/bin/llm \
  data/ordinary-usage/tools --model-port 18742 --binary /path/to/frozen/a2context
python3 examples/ordinary-usage/run.py papers /tmp/a2-papers/bin/python \
  data/ordinary-usage/papers --model-port 18742 --binary /path/to/frozen/a2context
python3 examples/ordinary-usage/review.py data/ordinary-usage/tools data/ordinary-usage/papers
```

Each run needs a new directory. Open its `review.md` for the sampled answers and exact model calls. The application logs, SQLite state and raw review materials stay in the checkout’s ignored `data/` directory. Sampling uses the trace IDs generated during the run. New inference can produce different outputs.
