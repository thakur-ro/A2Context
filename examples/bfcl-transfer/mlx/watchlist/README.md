# Does removing 15 tools change a watchlist update?

The full catalog and all three smaller-catalog conditions reached the requested database state on **all nine tasks**. They differed on one workflow. On task 132 the agent looked up the stock and added it to the watchlist in every condition. With 20 tools visible, it then answered the customer. With five, it issued an extra `get_watchlist` and used up its third completion. The extra read persisted when we padded the input back to the original length. [Protocol frozen before generation](protocol.md) · [Per-task measurements](results.json)

| Tools visible to the model | Exact workflow completed | Recovered / lost vs. full | Final state matched |
| --- | ---: | ---: | ---: |
| All 20 | 6/9 | Reference | 9/9 |
| Five permitted | 5/9 | 0 / 1 | 9/9 |
| Five, padded with periods | 5/9 | 0 / 1 | 9/9 |
| Five, padded with letters | 5/9 | 0 / 1 | 9/9 |

**What changed before inference?** On task 132, the tool-call opener had probability 99.87% with the full catalog and 99.94% with the short catalog. The full next-token distributions differed by 0.00012 bits of Jensen–Shannon divergence. The first proposed tool, both required calls, their arguments, and their returns matched. The workflows diverged on the third decision. Earlier generated text already differed between the full and short conditions, so the comparison leaves open whether catalog content or the intervening dialogue explains the extra read.

Task 116 illustrates the other direction. Letter padding lowered the opener probability from 99.98% to 86.22%. The model still made exactly the correct read and finished in every condition. A change in the opening-token score alone gives little guidance about whether to ship this edit.

## What counted as complete?

We selected every first turn in the pinned BFCL base set that used TradingBot alone, required a watchlist operation and referenced only permitted tools. Five tasks requested a list, four involved adding or removing a stock. Each fresh simulator had the same five permitted callbacks. Only the presented catalog and padding varied. Each padded input matched the full catalog’s token count on the same history. On the first turn we also checked identical tokens from the user-role marker through the answer boundary.

A pass required the exact reference calls and arguments in order, the reference return values with multiplicity, matching final public state, and normal completion within three generations. On task 142, for example, the model first looked up a symbol and then made the requested update. That extra lookup failed the exact-call criterion even though the state matched. The study does not grade final answer prose. It measures one greedy model, one workflow, and two specific fillers that also move tool definitions within the prompt. These nine cases cannot establish which catalog is generally better.

## Reproduce

Check the saved results anywhere with standard Python 3.11 or later. For fresh inference, use Apple Silicon and the pinned validator, MLX environment and checkpoint from the [model setup](../README.md#reproduce). Set `BFCL_CHECKOUT` to a writable data directory and `LFM_SNAPSHOT` to the checkpoint directory. `--fetch` downloads the public BFCL files at the pinned revision and checks their digests before importing the simulator. Use unused output filenames.

```sh
study=examples/bfcl-transfer/mlx/watchlist
python3 "$study/check.py"
HF_HUB_OFFLINE=1 python "$study/measure.py" --upstream "$BFCL_CHECKOUT" --fetch \
  --snapshot "$LFM_SNAPSHOT" --output /tmp/watchlist-first.json
HF_HUB_OFFLINE=1 python "$study/measure.py" --upstream "$BFCL_CHECKOUT" \
  --snapshot "$LFM_SNAPSHOT" --repeat /tmp/watchlist-first.json --output /tmp/watchlist-repeat.json
```

Each process ran all nine positive and nine no-call controls before free decoding. A run used 88 generations, 235,183 input tokens and 2,201 generated tokens, plus the separate first-token scoring passes. Two fresh processes matched on all recorded probabilities, prompts, generation hashes, callbacks and state digests. The receipts contain method names, counts and hashes. All callbacks ran against the upstream in-memory simulator. [Repeat receipt](verification.json) · [BFCL source at the tested revision](https://github.com/ShishirPatil/gorilla/tree/6ea57973c7a6097fd7c5915698c54c17c5b1b6c8/berkeley-function-call-leaderboard/bfcl_eval)
