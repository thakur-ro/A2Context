# Does changing the tool-call example produce a tool call?

The two native tool renderers changed the reply on **two of eight inputs**, both versions of the desk fan order. The tokenizer variant made the model name the exchange function in prose. Qwen-Agent still received no function call. Across all eight paired requests, switching renderers changed **zero order states**. [Raw responses and SQLite receipts](results.json)

| Outcome over eight requests | Qwen-Agent Nous | Qwen tokenizer |
| --- | ---: | ---: |
| Completed text replies | 8 | 8 |
| Parser emitted the expected tool name | 0 | 0 |
| Arguments passed native tool validation | 0 | 0 |
| Exact expected tool and arguments | 0 | 0 |
| Database matched expected state, gate off | 4 | 4 |
| Database matched expected state, gate on | 4 | 4 |

For the fan, the Nous response said “I'll submit the fan exchange.” The alternative said “I'll call the exchange_delivered_order_items function” and asked for IDs already present in the order lookup. The customer had approved the exchange. The registered tool received neither reply as a call, and both database receipts show the order still delivered. The hold cases had the expected database state under either renderer because no write occurred.

## What changed in the request

The experiment reused four synthetic orders, each with its stale and refreshed cache. It kept the customer message, consent, model, tool names, descriptions, argument schemas, parser, validator, checkpoint and approval policy fixed within each comparison. Qwen-Agent’s Nous renderer inserts its XML tool instructions into the system message. The alternate path passes the same effective schemas to the tools branch of the model’s own Transformers chat template. Both paths send the resulting assistant string to Qwen-Agent’s native Nous parser. [Frozen protocol](protocol.md) · [Qwen-Agent 0.0.34](https://pypi.org/project/qwen-agent/0.0.34/) · [Pinned Qwen tokenizer template](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/blob/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2/tokenizer_config.json)

Every pair differed in exactly two bytes in the system message’s example JSON. The Nous example wrapped the example in one pair of braces. The tokenizer template used two. That added two input tokens per request. We checked the complete prompt and token digests before decoding any of the alternate requests. Each fresh Nous output, including token IDs, matched the [preceding experiment](../prospective/README.md). [Eight frozen prompt pairs](design.json)

This result applies to one two-byte difference, one model, one parser and four previously examined orders. The two changed strings come from one order. The approved fan and grinder orders retained their original delivered status. Neither renderer produced the requested exchange in this cohort.

## Reproduce

Use Python 3.13, the pinned Qwen-Agent environment and the existing local snapshot. The first command checks every raw response against the real Nous parser and SQLite sandbox and runs four known-call controls without loading model weights. Adding the snapshot also verifies the complete rendered prompts, input tokens, output IDs and stop reasons. [Environment](../framework/README.md) · [Checkpoint](../../../public-qa/README.md#repeat-the-model-measurements)

```sh
study=examples/exchange-approval/dispatch-study/rendering
python "$study/measure.py" --check "$study/results.json"
python "$study/measure.py" --check "$study/results.json" --snapshot /path/to/qwen/snapshot
python "$study/measure.py" --snapshot /path/to/qwen/snapshot --output /tmp/renderer-results.json
python "$study/measure.py" --snapshot /path/to/qwen/snapshot \
  --repeat /tmp/renderer-results.json --output /tmp/renderer-repeat.json
```

The 16 fresh generations used 7,228 input and 350 output tokens in 15.69 seconds of decoding on four CPU threads. A separate process reproduced all eight tokenizer variants exactly in 9.54 seconds of decoding, using another 3,622 input and 187 output tokens. There were no paid calls or external tool dispatches. [Repeat receipt](verification.json)
