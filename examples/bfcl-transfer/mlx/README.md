# Can a second runner reproduce the travel workflows?

LiquidAI’s 8-bit LFM2.5-1.2B-Instruct completed four of the six first turns within the generation bounds. Two final states matched the reference. **None reproduced all required actions and responses.** The frozen gate therefore produced zero source-ranking probes and zero selected edits. Both runners failed this six-case test. Each used its own model, template and parser. The measurements cannot separate their contributions. [Frozen protocol](protocol.md) · [Per-call receipts](results.json)

| Task | What the LFM runner did | Expected state reached |
| --- | --- | --- |
| 150 | A fare lookup raised an error. The agent then called authentication and exchange-rate tools and reached the three-completion limit. | No |
| 155 | The booking failed schema validation. | No |
| 159 | A city lookup and a fare lookup returned, with different actions or arguments from the reference. | Yes |
| 163 | A fare and a budget lookup returned. The booking failed validation on the third completion. | No |
| 164 | A fare lookup raised a route error. | Yes |
| 179 | The booking failed schema validation. | No |

All nine reference callbacks ran through the MLX parser and shared argument validator. All six positive controls passed. No-call controls missed the reference actions and results on every task. The two unchanged runs matched on prompts, generated text, callback dispositions, returns and state. Each process used 15 generations, 68,313 input tokens and 838 output tokens. Generation took 22.29 seconds in the first process and 21.38 in the second on this host. [Independent repeat](verification.json)

## Reproduce

Use the upstream checkout described in the original study and the same Qwen-Agent argument-validator environment. On Apple Silicon add `mlx==0.32.4`, `mlx-lm==0.32.2` and the Transformers version used by this MLX environment, `5.23.0`. Download the MLX checkpoint at revision `ce5150fd0de58d2bba8d88cc87f13e4fbda5977a` separately. The runner verifies the eight local file digests before loading weights. All tools run against a fresh in-memory simulator. [Model checkpoint](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit/tree/ce5150fd0de58d2bba8d88cc87f13e4fbda5977a) · [Upstream data and validator setup](../README.md#reproduce)

```sh
study=examples/bfcl-transfer/mlx
python "$study/check.py"
HF_HUB_OFFLINE=1 python "$study/calibrate.py" --upstream "$BFCL_CHECKOUT" \
  --snapshot "$LFM_SNAPSHOT" --output /tmp/lfm-travel.json
HF_HUB_OFFLINE=1 python "$study/calibrate.py" --upstream "$BFCL_CHECKOUT" \
  --snapshot "$LFM_SNAPSHOT" --repeat "$study/results.json" --output /tmp/lfm-repeat.json
```

The first command checks the archived evidence with standard Python. The others run local inference and require unused output paths.
