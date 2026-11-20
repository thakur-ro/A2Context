# Can the unchanged request reproduce the recorded proposal with more output space?

Four of the five unchanged Qwen requests exhausted their 128-token budget in the initial transfer study. Its one complete baseline was a text reply to a checkpoint whose recorded next action was a human transfer. This follow-up uses the same four censored prompts. The single changed generation setting is a limit of 256 new tokens. Run each baseline once, with the same model files, tokenizer, template, CPU float32, greedy decoding and four model threads. Accept the comparison only if the first 128 generated tokens decode to the exact saved output digest for that task.

Count newly completed proposals, tool names and arguments matching a recorded next tool call, replies that agree only in response kind, and still-censored outputs. For a completed plain-text reply to a historical text turn, content equivalence is unknown. A historical next event is an observed behavior and may be one of several acceptable responses. This test has no ground-truth label for the best next action.

Stop after the four unchanged requests. Before spending more on context edits intended to explain a specific historical action, require a completed baseline that reproduces that action at the requested level of detail. Keep outcomes at 128 tokens in the original study. The longer-budget comparison was chosen after seeing its truncations.

```sh
python3 examples/airline-transfer/baseline_budget.py --data /tmp/airline-study --snapshot /path/to/qwen/snapshot --output /tmp/baseline-budget.json
python3 examples/airline-transfer/baseline_budget.py --check /tmp/baseline-budget.json
```

## Result

All four longer runs reproduced the saved 128-token prefix exactly. Task 29 still reached the new limit. Task 30 ended at 218 tokens, task 32 at 233 and task 37 at 234. All three completed with plain text. The recorded agent had proposed a reservation update on tasks 30 and 37, and three flight searches on task 32. Together with the unchanged task 44 text reply from the first run, the local runner produced text on all four historical tool-call checkpoints. Expanding the output budget recovered three completed replies and zero historical tool proposals. Further source edits on this runner cannot yet explain those recorded actions. [Four paired continuations](baseline-budget.json) · [Original five baselines](results.json)
