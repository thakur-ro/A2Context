# Can a local agent reproduce the starting point of a travel workflow?

On six first-turn tasks from Berkeley’s Function Calling Leaderboard, local Qwen finished every interaction and attempted a travel function on every task. **Four of six simulator states matched the reference. None of the six reproduced all required actions and their results.** The registered baseline check therefore stopped the experiment before source ranking. No edit or ranking method was evaluated on these cases. [Per-case receipts](results.json) · [Ranking gate](ranking.json)

| BFCL task | Reference first-turn actions | Local attempt | Final state matched |
| --- | --- | --- | --- |
| 150, lookup | Fare lookup | Supplied different origin and destination. The simulator raised a route error | Yes |
| 155, booking | Fare lookup, book | Booked with the exact reference arguments but skipped the fare lookup | Yes |
| 159, lookup | List airports, fare lookup | Skipped the airport list and requested an unavailable route | Yes |
| 163, booking | Fare lookup, book | Tried to book. The native argument validator rejected the call | No |
| 164, lookup | Fare lookup | Supplied different origin and destination. The simulator raised a route error | Yes |
| 179, booking | Book | The simulator returned a booking error for arguments with different origin and destination | No |

Task 155 shows why the gate checks more than a database snapshot. Qwen produced the right booking and the simulator ended in the reference state. The published first turn also called for checking the fare first. Of the nine required calls across the six tasks, the exact booking on 155 was the only one completed with the reference arguments. On three read-only tasks, a plain text response with no call would also have left the correct public state. [Native replay diagnoses](diagnostics.json)

## Checkpoints and scoring

The public V4 base dataset contains 200 multi-turn tasks. Thirteen use only the in-memory TravelAPI. We removed three whose first-turn reference involved identity verification that depends on today's date, then took the first three reference turns that changed public state and the first three that preserved it. All six started from distinct public-state digests. The state includes booking and account records. BFCL's TravelAPI equality omits its private flight-price cache and random generator. Each tuple of task, turn, schema set, initial state and reference action was frozen before any local model output. [Selection](selection.json) · [Protocol](protocol.md)

The local agent received the exact benchmark first user message, one staging system instruction and all 18 published travel function schemas. For the tool framework, the official schema types `dict` and `float` become `object` and `number`. Qwen-Agent's Nous formatter and parser handled raw local greedy completions without assistant prefilling. Its registered tools ran the pinned upstream Python TravelAPI methods on a fresh isolated object. The initial database, reference arguments, future user turns and outcomes stayed outside model context. Generated tool responses were fed back to the model.

The state check uses TravelAPI’s native equality. The reference-action check requires each published name and arguments in order among executed calls. The result check requires each reference return value with multiplicity. These tests are stricter than the upstream leaderboard’s state and execution-response checks. This is a six-case calibration for the NexusWeave source-ranking experiment. It covers one local checkpoint, one tool presentation and a single first turn per task. The public benchmark’s presence in model training data is unknown. [Official evaluator](https://github.com/ShishirPatil/gorilla/blob/6ea57973c7a6097fd7c5915698c54c17c5b1b6c8/berkeley-function-call-leaderboard/bfcl_eval/eval_checker/multi_turn_eval/multi_turn_checker.py) · [BFCL paper, Patil et al., ICML 2025](https://proceedings.mlr.press/v267/patil25a.html)

## Reproduce

Berkeley’s repository and dataset card identify Apache-2.0. The loader fetches eight pinned upstream files, including both notices, and verifies each SHA-256 before loading the in-memory simulator. The tracked receipts contain method names, field names and digests. Simulator accounts, customer text and private raw model completions stay outside the repository. [Upstream license](https://github.com/ShishirPatil/gorilla/blob/6ea57973c7a6097fd7c5915698c54c17c5b1b6c8/LICENSE) · [Data card](https://github.com/ShishirPatil/gorilla/blob/6ea57973c7a6097fd7c5915698c54c17c5b1b6c8/berkeley-function-call-leaderboard/bfcl_eval/data/README.md) · [Hashes](sources.json)

From the repository root, reconstruct the selection, replay the nine known reference calls through the native validator, and run six no-call controls. Only the input fetch requires internet access. The model-free checker needs the [pinned Qwen-Agent environment](../exchange-approval/dispatch-study/framework/README.md).

```sh
study=examples/bfcl-transfer
data=/tmp/nexus-bfcl-data
python "$study/inputs.py" "$data" --fetch > /tmp/nexus-bfcl-selection.json
cmp /tmp/nexus-bfcl-selection.json "$study/selection.json"
python "$study/run.py" --upstream "$data" --check "$study/results.json"
```

To reproduce the local measurements, use the same [CPU libraries and checkpoint](../public-qa/README.md#repeat-the-model-measurements). The audit file retains raw generated strings locally. The checker replays those through Qwen-Agent and the simulator and verifies their hashes and the rendered model prompts. Output destinations must be unused.

```sh
snapshot=/path/to/qwen/snapshot
python "$study/run.py" --upstream "$data" --snapshot "$snapshot" \
  --output /tmp/bfcl-results.json --audit /tmp/bfcl-private-audit.json
python "$study/run.py" --upstream "$data" --snapshot "$snapshot" \
  --check "$study/results.json" --audit /tmp/bfcl-private-audit.json
python "$study/run.py" --upstream "$data" --snapshot "$snapshot" \
  --repeat /tmp/bfcl-results.json --output /tmp/bfcl-repeat.json
python "$study/run.py" --upstream "$data" --rank /tmp/bfcl-results.json \
  --verification /tmp/bfcl-repeat.json --output /tmp/bfcl-ranking.json
python "$study/diagnose.py" --upstream "$data" --results "$study/results.json" \
  --audit /tmp/bfcl-private-audit.json | cmp - "$study/diagnostics.json"
```

Each model run used 12 generations, 37,227 input tokens and 629 generated tokens, with no truncations, cap exits or paid API calls. The main run took 69.92 seconds of decoding on four CPU threads. A separate process reproduced every prompt, output, call and database receipt in 68.12 seconds. All six hand-written reference trajectories passed both the required-action and final-state checks. Every no-call control missed the required actions and results. [Verification receipt](verification.json)
