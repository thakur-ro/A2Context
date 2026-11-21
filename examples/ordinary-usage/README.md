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

The project owner delegated validation to Codex. Review against the attached database and frozen source documents found concrete issues in five of the six sampled quiet sessions. Codex also implemented the pilot. This was a developer review on one small local model and one person’s engineering questions. [Six dispositions and supporting evidence](quiet-review.md) · [Per-request record and application output digests](first-pass.json)

We tested one possible explanation after the pilot. One tool description referred to an unavailable tool. Correcting the description produced fewer first-step tool proposals on the same inputs. [Controlled edit, including unchanged repeats](catalog-edit.md)

Two further tests looked for useful context in earlier sessions. In PaperQA, the search recovered a relevant formula and traced its omission to a summary. Passing the whole source passage to the final model still yielded an incorrect explanation at both tested seeds. In SQLite, adding a verified description of the database failed to elicit a tool proposal on the one eligible later question. Run `python3 examples/ordinary-usage/check.py` to verify published counts and evidence references offline. [Retrieval and summary experiments](retrieval-check.md) · [SQLite schema comparison](earlier-schema.md)

## What the revised feed found

When we played the same requests through the revised sidecar, it surfaced two distinct recommendations. In four SQLite conversations, the reply described a tool limitation while the request offered a schema-discovery tool. A separate conversation with the same catalog contained an invocation and result. An engineer can use those calls to check whether the application gave the agent access to the intended database.

In all five PaperQA conversations, a provider reply cited an identifier that appeared in a prompt formatting example and had no matching captured passage label. In 14 of 15 matched trials, both unchanged replies cited an original example key and the edited reply cited its replacement. [Experiment and individual results](citation-examples.md)

The automatic checks used zero model calls. Both were developed after reviewing this cohort. Four SQLite occurrences count as one recommendation, and five PaperQA occurrences count as one. The original frozen pilot remains the measure of performance on these applications before tuning.

Resending all 28 spans to an empty database surfaced the SQLite finding after the first span and the citation finding after the twelfth (PaperQA’s fourth call). Both evidence links resolved to the recorded model call. We checked the printed PaperQA answers as well. Seven example-only citations became empty parentheses across the five answers. The feed still missed the off-topic budget answer and the incorrect explanation of how missing capture affects the score. [Displayed citations](displayed-citations.json) · [Quiet-session review](quiet-review.md)

We then froze the revised detector and connected two different workflow implementations. It again showed zero recommendations. Four LangChain sessions ended with a rejected tool proposal. One LlamaIndex answer copied an irrelevant topic from a prompt example. [Transfer pilot and review of the quiet sessions](transfer/README.md)

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

To inspect the revised feed, start the current binary on loopback with a new database and content capture enabled. Run `python3 examples/ordinary-usage/replay_feed.py data/ordinary-usage --port PORT` against that instance. The script resends saved spans in completion order, queries the feed after each export, and resolves the displayed evidence to its model call. No inference is needed.
