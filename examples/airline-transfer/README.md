# Which source edit changes an airline agent's next proposal?

Masking the last customer message changed the local model’s first-token distribution most on **all five requests**. A rule that always picks the last customer message selected the same edit. The experiment could score the effect on the final action for only **one of the five**. Four unedited responses reached the output limit before the model finished. [All 29 measurements](results.json)

| Test task | Published next event | Highest divergence, bits | Qwen, unedited | Qwen, after withdrawal |
| --- | --- | ---: | --- | --- |
| 29 | Text reply | 0.382 | Truncated | Truncated |
| 30 | Update reservation flights | 0.569 | Truncated | Text reply |
| 32 | Three flight searches | 0.780 | Truncated | Text reply |
| 37 | Update reservation flights | 0.358 | Truncated | Text reply |
| 44 | Transfer to a human | 0.480 | Text reply | Text reply |

For task 44, the ranked edit and all three baselines left the proposal category unchanged. The policy, reservation lookup and customer-message edits changed the exact text. The latest tool-result edit reproduced it verbatim. We did not score reply meaning. Across the five requests, the ranked edit, policy-first, latest-message and latest-tool-result selectors each had **0 changes in 1 scorable comparison**, for both write status and canonical action. The four incomplete comparisons carry no outcome label. The local model reproduced none of the four published tool-call events, with three of those local baselines truncated.

Across all arms, 13 generations ended with a text reply and 16 reached the 128-token limit. No completed generation proposed a tool call. Four withdrawal requests yielded a complete nonwrite and one truncated. No unedited baseline had a confirmed write to reverse. Under this configuration, a high divergence identifies an edit worth testing. The observed action coverage is too low to recommend that someone change a source to alter a booking outcome.

The main run used four CPU threads. The 24 first-token measurements took 258 seconds over 143,013 input tokens. The 29 greedy generations took another 470 seconds over 173,031 input and 2,601 generated tokens, counting EOS where present. There were no external model calls. These are offline replay costs for prompts of roughly 5,000–6,800 tokens. [Measured costs and stop reasons](results.json)

An independent process re-rendered all 29 requests and rechecked the saved raw output digests, parser dispositions and five historical events. A fresh run of task 29 reproduced all six completion digests and dispositions, all four divergence scores and their ordering exactly. That repeat used five probes and six generations with 708 output tokens. [Verification receipt](verification.json)

## Where the requests came from

Sierra's published Claude Opus 4.5 airline submission contains 200 agent trajectories covering 50 tasks and four trials. The simulation file records the policy, agent and customer messages, tool-call IDs and tool results. We used trial zero on five tasks from the official test split. The checkpoint manifest gives the last included message index, its timestamp, the next event's timestamp and digests of the model-visible history and each edited source. The agent's next event, hidden simulator instructions, benchmark references, evaluator results and private reasoning stayed outside the local model input. [Leaderboard submission](https://github.com/sierra-research/tau2-bench/blob/c06b07dd7c4d4ec61ba6ff9bcdf0b15dd7ef36d4/web/leaderboard/public/submissions/claude-opus-4-5_sierra_2026-02-26/submission.json) · [Frozen checkpoints](selection.json)

The five tasks have a lookup and a cancellation or update in their benchmark reference actions. The reference names and arguments agree across the two pinned sources. Reference actions describe one way to complete the whole task. Several next steps can satisfy that task. Two original next events were reservation writes with exact arguments matching a task reference. The other three were a human transfer, a batch of three read-only searches and a text response. These five public test tasks were new to this project’s source-ranking experiments. Exposure of either model to similar benchmark material is unknown. [Official evaluation semantics](https://github.com/sierra-research/tau2-bench/blob/c06b07dd7c4d4ec61ba6ff9bcdf0b15dd7ef36d4/docs/evaluation.md) · [Protocol](protocol.md)

The public file omits the provider request, base instructions and tool schemas. We reconstructed the Qwen chat using the recorded policy and public transcript. For its 14 tools, we took names, top-level required arguments, primitive types and operation categories from Sierra's pinned Python source. We supplied the first line of each tool's docstring as its description. Arrays of objects have no nested field constraints in this reconstruction. We can check the proposed action category and exact argument digest. We did not execute a reservation change or score an end-to-end benchmark task. [Pinned sources and hashes](sources.json)

We also screened the separate Exgentic airline corpus at revision `70036b93a04e61b0ea2706a68b962f4f26774587`. All 341 spans in its 59 airline sessions labeled `tool_calling` were customer simulator calls. None included a tool definition, linked tool result or output tool call. The top-level harness label describes the run, which is insufficient to identify which participant made a recorded call. [Census](exgentic-census.json) · [Exgentic data card](https://huggingface.co/datasets/Exgentic/agent-llm-traces/blob/70036b93a04e61b0ea2706a68b962f4f26774587/README.md)

## Reproduce

The τ²-Bench study evaluates agents across full conversations in simulated environments. This transfer experiment fixes the conversation prefix and varies one source before the next proposal. [Barres et al., ICML 2026](https://proceedings.mlr.press/v306/barres26a.html)

The results contain tool names and digests. Downloaded transcripts and optional raw Qwen generations stay outside the checkout. The input loader verifies each file's SHA-256 digest and reads the public Python tool signatures as syntax without executing that file. From the repository root, fetch and reconstruct the five requests. Add `--census` to fetch and screen the five Exgentic shards using PyArrow (tested with 21.0.0).

```sh
python3 examples/airline-transfer/inputs.py /tmp/airline-study --fetch
python3 examples/airline-transfer/inputs.py /tmp/airline-study > /tmp/airline-selection.json
cmp /tmp/airline-selection.json examples/airline-transfer/selection.json
```

To independently locate the airline rows, run `inputs.py /tmp/airline-study --discover-shards` in a separate census environment with `huggingface_hub==1.8.0` and `pyarrow==21.0.0`. This reads the benchmark column across all 39 pinned shards and verifies that only shards 20 through 24 contain the airline benchmark. Then run `inputs.py /tmp/airline-study --fetch --census` to download those five shards and reproduce the recorded span counts.

Python 3.11 or later can verify the saved summaries and check historical next-event signatures against the downloaded trajectory. Fresh scoring also requires the pinned Qwen checkpoint and CPU libraries. [CPU setup and checkpoint](../public-qa/README.md#repeat-the-model-measurements)

```sh
python3 examples/airline-transfer/run.py --check examples/airline-transfer/results.json --data /tmp/airline-study

python3 examples/airline-transfer/run.py --data /tmp/airline-study --snapshot /path/to/qwen/snapshot \
  --output /tmp/airline-results.json --audit /tmp/airline-private-raw.json
python3 examples/airline-transfer/run.py --check /tmp/airline-results.json --data /tmp/airline-study \
  --snapshot /path/to/qwen/snapshot --audit /tmp/airline-private-raw.json
```

The last command checks every rendered prompt and token count with the local tokenizer, then parses every private output again. Add `--repeat-task 29` to regenerate all six arms of that checkpoint and compare the ranks, divergence scores, completion hashes, stop reasons and proposed actions.
