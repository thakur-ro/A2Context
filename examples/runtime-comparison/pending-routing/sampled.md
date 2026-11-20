# Repeat a routing edit across sampling seeds

This pilot uses one synthetic catalog request for product 28. The source edit changes “Request type: stock.” to “Request type: price.” For each of four seeds, the runner generates an original and an edited proposal under two instructions and two tool inventories. The seed resets before every generation, so both arms of a comparison start with the same random seed. A changed tool name and a changed complete choice are separate outcomes. [Fixed protocol](sampled-protocol.json) · [Instructions and tool definitions](routing-design.json)

The runner uses CPU float32, four PyTorch threads, temperature 0.7 and unrestricted multinomial sampling. It stops after 48 new tokens. A valid tool choice retains the parsed tool name and arguments. A text choice retains the decoded answer. A missing end token, unparseable tool proposal or runner error leaves the arm failed. Each retained pair gives one observation under its seed and setup. Four seeds on one input supply limited evidence about repeatability.

## Observed outcomes

All 32 proposal trials completed. A second execution on the same machine reproduced the complete result file byte for byte. In the original instruction with both tools available, the source edit changed the complete choice for three seeds and changed the selected tool for two. Seeds 0 and 1 switched from `stock` to `price`. Seed 2 produced two text answers that differed in wording. Seed 3 proposed `stock` in both arms. Every proposed tool call named product ID 28.

| Instruction | Available tools | Complete choice changed | Tool selection changed |
| --- | --- | ---: | ---: |
| Original | stock | 0/4 | 0/4 |
| Original | stock, price | 3/4 | 2/4 |
| Alternate | stock | 0/4 | 0/4 |
| Alternate | stock, price | 4/4 | 4/4 |

The denominator is four paired seeds on the same input. The runner scored the original request and its two edits once, then reused that report across the four seed records. The initial score on that input was a Jensen–Shannon divergence of 0.002370 bits for the request-type edit and 0.00005729 bits for the store-name edit, computed from the model's first-token logits before temperature scaling. Only the request-type edit was tested during generation. [Recorded choices and request digests](sampled-results.json)

## Reproduce

Use Python 3.13, Transformers 4.57.6 and PyTorch 2.11.0 with a local copy of the pinned Qwen model. The model weights used here had SHA-256 `fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe`. [Qwen2.5-0.5B-Instruct weights and checksum](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/blob/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2/model.safetensors)

```sh
cargo build --locked
study=examples/runtime-comparison/pending-routing
python3 "$study/sampled_routing.py" --model /path/to/qwen/snapshot --output /tmp/sampled-routing.json
cmp "$study/sampled-results.json" /tmp/sampled-routing.json
```

To inspect the saved records in a running sidecar with content capture enabled, export them over OTLP. Select organization `routing-sampling`. The four records share one conversation ID and retain the synthetic timestamp of the source fixture. The run IDs identify their sampling seeds.

```sh
python3 examples/runtime-comparison/pending-routing/sampled_routing.py | \
  curl --fail-with-body -H 'Content-Type: application/json' --data-binary @- http://127.0.0.1:8040/v1/traces
```
