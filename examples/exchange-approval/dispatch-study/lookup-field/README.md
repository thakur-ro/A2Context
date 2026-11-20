# Which source supplied the payment argument?

Changing one digit in the order lookup changed exactly one token in the generated payment argument on **four of four orders**. Changing one punctuation token in an earlier customer message left **all four complete outputs unchanged**. The tool name and the order, item and replacement IDs stayed identical in every pair. All requests used the same caller-supplied tool-call opener. [Full completions and receipts](results.json) · [Frozen positions and prompt digests](design.json)

| Order | Authentic → substituted lookup | Input token index | Generated suffix token index |
| --- | --- | ---: | ---: |
| Electric kettle, held | `gift_card_8213` → `gift_card_8214` | 115 | 67 |
| Desk fan, approved | `card_4927` → `card_4928` | 114 | 66 |
| Coffee grinder, approved | `paypal_1538` → `paypal_1539` | 114 | 66 |
| Camping tent, held | `card_6172` → `card_6173` | 114 | 66 |

Indices begin at zero. In every targeted pair the input and output token IDs changed from the same original digit to the same new digit. This locates a causal effect of changing this captured field on this generated argument under the tested intervention. The experiment measured a token sequence and a dispatch result. It did not observe which internal representations carried the information.

## What happened to the write?

The substituted IDs had valid syntax, and Qwen-Agent’s tool validator accepted every call. The application still required the original four identifiers. Its in-memory SQLite dispatcher rejected **all four** substituted payment IDs as `invalid_arguments`, before the approval check, and changed no row. The two approved orders lost their expected writes under either gate policy. The two held orders stayed delivered because of the argument rejection. With the gate off, that happened to restore their required state; the model still proposed an exchange against the customers’ holds. The punctuation controls reproduced the original proposals and state receipts under both policies.

## Check before and after inference

The saved pending envelopes pin the expected payment text digest from the independent application design. Its source pointer covers exactly the payment ID in the model-facing system message. The trusted digest and source labels are capture metadata. Neither is part of the model prompt. The example below opens the targeted fan request in the Rust preflight. Its source coverage reports a text mismatch for `payment-lookup`. In this cohort, all four targeted requests produced that mismatch. All eight original and punctuation-control requests matched their pinned digests. [Ready-to-load requests](pending-requests.json)

```sh
study=examples/exchange-approval/dispatch-study/lookup-field
jq -c '.cases[1].arms.lookup' "$study/pending-requests.json" |
  target/debug/nexusweave preflight -
```

The punctuation control matches the target edit’s byte and token count. Its position, token class and meaning differ. The four cases were previously measured under a forced tool opener, and every target used the repaired consent-cache condition. This comparison does not measure natural tool selection or identify the most important source among unseen alternatives. [Protocol](protocol.md) · [Prior prefix experiment](../prefill-tool/README.md)

## Reproduce

Use the [pinned framework environment](../framework/README.md) and [CPU checkpoint](../../../public-qa/README.md#repeat-the-model-measurements). The first command replays 12 parser inputs, checks 24 SQLite receipts and runs 16 known-call validation controls without model weights. With the local tokenizer it also verifies all message byte offsets, token spans and generated strings. The token-differences view prints the input and output IDs directly from the saved trials.

```sh
python "$study/run_lookup.py" --check "$study/results.json"
python "$study/run_lookup.py" --check "$study/results.json" --snapshot /path/to/qwen/snapshot
python "$study/run_lookup.py" --token-differences "$study/results.json"
python "$study/edits.py" /path/to/qwen/snapshot | cmp - "$study/design.json"
python "$study/run_lookup.py" --snapshot /path/to/qwen/snapshot --output /tmp/lookup-results.json
python "$study/run_lookup.py" --snapshot /path/to/qwen/snapshot \
  --repeat /tmp/lookup-results.json --output /tmp/lookup-repeat.json
```

The 12 main runs used 5,433 input tokens, including 24 caller-supplied assistant tokens, and generated 843 tokens in 30.81 seconds on four CPU threads. A separate process reproduced all eight edited generations and both sets of receipts using 3,622 input and 562 generated tokens in 20.50 seconds. No paid model or external service received a request. [Repeat receipt](verification.json)
