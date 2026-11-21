# Did refreshing consent fix the exchange?

Refreshing the stale consent record cleared the declared conflict on **four of four** fresh synthetic orders. It changed the pending order on **zero of four**. The local model answered in ordinary text on all eight runs. Qwen-Agent received no tool call, and neither approved exchange reached the database. Both held orders stayed delivered as requested. [Full requests, completions and receipts](results.json) · [Frozen protocol](protocol.md)

| Pending exchange | Customer asked | Exact response changed? | Native tool, before → after | Pending order, before → after |
| --- | --- | --- | --- | --- |
| Electric kettle | Hold | No | None → none | Delivered → delivered |
| Desk fan | Approve | No | None → none | Delivered → delivered |
| Coffee grinder | Approve | No | None → none | Delivered → delivered |
| Camping tent | Hold | Yes | None → none | Delivered → delivered |

For the desk fan, the model said “Sure, I'll submit the fan exchange. Please wait a moment while I process it.” The cache refresh produced the same reply, byte for byte. Both database receipts leave the pending order delivered and the previous exchange requested. The expected tool was `exchange_delivered_order_items` with four specific IDs. For the held orders the application expected a `message` tool call. The held orders had the expected state even though the model missed that tool. The two gate policies agreed across all eight receipts because there were no write proposals to gate.

## Inspect the same request

Build the CLI with `cargo build --locked`. From the repository root, inspect the desk fan’s original request, then change `original` to `repair` and run it again.

```sh
study=examples/exchange-approval/dispatch-study/prospective
jq -c '.cases[1].arms.original.pending_request' "$study/results.json" |
  target/debug/a2context preflight -
```

The original result has one conflict for order `#R831649`. Its anchors identify the cached hold and the customer’s approval. The repaired result has zero conflicts. In the same fixture, the parser and database receipts show what happened after inference.

```sh
jq '.cases[1] | {expected, arms: (.arms | map_values({
  reply: .generation.raw, parser, before: .with_gate.before, after: .with_gate.after
}))}' "$study/results.json"
```

## What the fixture captures

- `workflow_request` contains the application’s messages and complete structured tool declarations. Qwen-Agent’s native Nous formatter converts the supported schema fields into text in the system message.
- `pending_request` contains the actual post-framework messages handed to the local tokenizer, the captured cache and customer byte ranges, and the model settings. The tool signatures are already inside those messages.
- `generation` contains the complete rendered prompt, its text and token digests, input count, raw response, output IDs, stop reason and elapsed decoding time. The framework’s native `FnCallAgent` received that raw response.
- `parser` records any native call, validated arguments and agreement with the exact application target. Each gate receipt includes a SQLite read of an independently initialized copy of the starting checkpoint and the state returned after the trial. A text reply leaves both orders untouched. This staging harness stops after the first completed response or tool callback.

The experiment used four new order pairs with deliberately stale caches and application-supplied consent labels. It tested a single cache refresh selected from the earlier conflict, before any fresh model output. Real stale-cache frequency and the reliability of extracting consent from customer language remain open questions. [Earlier exchange](../../../../docs/demo.md) · [Qwen-Agent](https://github.com/QwenLM/Qwen-Agent)

The checker also injects a hand-written valid exchange call for one held and one approved order. Both reach the real tool validator. SQLite writes both in the ungated sandbox, while the enforced sandbox blocks the held order and writes the approved one. These four plumbing checks are separate from the eight generated responses.

## Reproduce

Use Python 3.13 and the framework study’s pinned dependencies. The saved completions can be replayed without the model weights. Add the local snapshot to check every rendered prompt, input-token digest and output decode. An independent process regenerated all four original completions and matched the prompts, parser outcomes and SQLite receipts exactly. [Framework environment](../framework/README.md) · [Snapshot and CPU setup](../../../public-qa/README.md#repeat-the-model-measurements) · [Repeat receipt](verification.json)

```sh
python3.13 -m venv /tmp/exchange-prospective
. /tmp/exchange-prospective/bin/activate
python -m pip install -r "$study/../framework/retry-requirements.txt"
python "$study/run.py" --check "$study/results.json"
python "$study/run.py" --check "$study/results.json" --snapshot /path/to/qwen/snapshot
python "$study/run.py" --snapshot /path/to/qwen/snapshot --output /tmp/prospective.json
python "$study/run.py" --snapshot /path/to/qwen/snapshot \
  --repeat-baselines /tmp/prospective.json --output /tmp/prospective-repeat.json
```

The eight primary generations used 3,606 input and 163 output tokens and took 7.36 seconds of decoding on four CPU threads. The four baseline repeats used another 1,803 input and 84 output tokens in 3.74 seconds. There were no paid calls, network exporters, real customers or production writes.
