# Did a verified airport directory fix the travel calls?

No first turn completed all the required actions in any condition. The directory supplied two airport codes before inference on four of six requests. On the four changed prompts, the verified and withheld conditions produced identical first completions on three tasks. On the fourth, the proposed booking arguments changed, but both attempts failed schema validation. [Generation and simulator receipts](results.json) · [Frozen protocol](protocol.md)

| BFCL task | Locations matched in the user message | Unchanged request | Directory with codes withheld | Directory with verified codes |
| --- | ---: | --- | --- | --- |
| 150 | 2 | Fare lookup raised a route error | Called a city lookup and stopped before checking the fare | Same call and output as withheld |
| 155 | 0 | Correct booking, skipped fare lookup | Original input reused | Original input reused |
| 159 | 0 | Unavailable fare route, skipped airport list | Original input reused | Original input reused |
| 163 | 2 | Booking failed schema validation | Booking failed schema validation | Different proposed arguments, same rejection |
| 164 | 2 | Fare lookup raised a route error | Changed fare arguments, still a route error | Same call and output as withheld |
| 179 | 2 | Booking returned a business error | Same validated booking arguments and error | Same call and output as withheld |

We chose this test after inspecting the original failures. Eight proposed airport fields used unsupported values. The simulator could resolve seven of those values to the expected codes. For the intervention we looked up phrases **only from the pending customer message**. Task 159 illustrates the difference. The lookup could resolve the two values in the later proposal, yet found zero locations in the message available before inference. [Replayed field diagnostics](baseline-fields.json)

All three conditions reached four of six expected final states. Three were read-only tasks. On task 150, adding a directory with *either* content changed the tool choice relative to the original. Comparing only the enriched and original prompts would have credited the supplied codes for a change that also occurred when the values were withheld. On task 163 the model was sensitive to the values, but the resulting proposal still failed. This experiment measures a prompt intervention on one model and six previously examined cases. The cases provide no basis for estimating production accuracy or identifying a hidden reasoning path.

## Reproduce

Use the same pinned upstream files, model snapshot and Qwen-Agent CPU environment as the original study. The lookup uses the simulator's local read-only methods. The receipts contain hashes, call names and statuses. The control replaces each verified code with the literal `[withheld]`, which changes token length. [Upstream setup and attribution](../README.md#reproduce)

```sh
study=examples/bfcl-transfer/airport-context
python "$study/check.py"
python "$study/study.py" --upstream "$BFCL_CHECKOUT" --snapshot "$QWEN_SNAPSHOT" \
  --output /tmp/bfcl-airport-context.json
python "$study/study.py" --upstream "$BFCL_CHECKOUT" --snapshot "$QWEN_SNAPSHOT" \
  --repeat "$study/results.json" --output /tmp/bfcl-airport-repeat.json
```

The first command checks the saved counts and digests with Python 3.10 or later. To reconstruct the initial field diagnostic, run the original study's `diagnose.py` command with `--airport-fields` and compare its output with `baseline-fields.json`.

Each process made 16 fresh local generations, with 49,876 input tokens and 655 generated tokens. Decoding took 85.13 seconds in the first run and 86.07 seconds in the second. The repeat reproduced the prompt, output, callback and state receipts for every freshly generated arm. The two zero-match cases reused their previously repeated baselines. [Independent repeat](verification.json)
