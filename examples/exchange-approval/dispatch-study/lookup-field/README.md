# Which source supplied the payment argument?

Changing the last digit of the payment identifier in the order lookup changed the matching digit in the generated tool call on all four orders. The tool name, three other arguments and remaining generated tokens stayed identical on each order. A one-token punctuation change in an earlier user message left all four output sequences unchanged. [Frozen design](protocol.md) · [Per-request outputs](results.json) · [Independent repeat](verification.json)

The four requests used Qwen2.5-0.5B-Instruct, a refreshed consent record and a caller-supplied tool-call opener. Each lookup edit changed one input token without shifting the following tokens. We tested whether that field could control the payment argument after the caller had already started a tool call. The experiment leaves open how the model chose the response format on a free start.

| Input | Schema-valid calls | Calls with authentic payment ID | Orders in expected state with approval check |
| --- | ---: | ---: | ---: |
| Original | 4/4 | 4/4 | 4/4 |
| One lookup digit changed | 4/4 | 0/4 | 2/4 |
| Punctuation control | 4/4 | 4/4 | 4/4 |

The simulator checked every call against the original order. It rejected all four substituted identifiers. Both approved exchanges remained unsubmitted. Both held orders stayed delivered even though the model proposed an exchange. The punctuation control matches the lookup edit in token count and changes a different position and token class. This cohort contains four previously examined synthetic orders.

## Flag the substitution before decoding

The workflow supplies a digest of the payment ID from the upstream order and anchors that field in the model-facing system message. The expected digest is identical across the three variants of each order. The CLI and the sidecar flagged all four substitutions and passed the eight original and punctuation-control requests. This check requires a trusted reference value and makes zero model calls.

From the repository root, run the desk fan substitution through preflight.

```sh
study=examples/exchange-approval/dispatch-study/lookup-field
cargo build --locked
jq '.cases[] | select(.id=="fan-approved") | .arms.lookup' "$study/pending-requests.json" |
  target/debug/nexusweave preflight - | jq '.source_coverage'
```

To inspect that request in the dashboard, save the file below and open it in **Before inference**. **Inspect differing fragments** shows the source, byte range and digests. Change `.arms.lookup` to `.arms.punctuation` to check the control.

```sh
jq '.cases[] | select(.id=="fan-approved") | {request: .arms.lookup}' \
  "$study/pending-requests.json" > /tmp/fan-payment.json
```

## Reproduce the measured calls

Use the preceding study's pinned Python environment. The first command replays the native Qwen-Agent parser and SQLite receipts without model weights. The second checks the rendered prompts and token IDs against the local snapshot. A new measurement uses an unused output path. [Framework dependencies](../framework/README.md) · [CPU model setup](../../../public-qa/README.md#repeat-the-model-measurements)

```sh
python "$study/run_lookup.py" --check "$study/results.json"
python "$study/run_lookup.py" --check "$study/results.json" --snapshot /path/to/qwen/snapshot
python "$study/run_lookup.py" --snapshot /path/to/qwen/snapshot --output /tmp/payment-trials.json
python "$study/run_lookup.py" --snapshot /path/to/qwen/snapshot \
  --repeat /tmp/payment-trials.json --output /tmp/payment-repeat.json
```
