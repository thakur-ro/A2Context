# Why the inventory replay returned 2 instead of 42

The workflow recorded a stock lookup in the assistant’s structured `tool_calls` field and a tool result containing 42 units. The two 8-bit LFM2.5 packages rendered that request differently. The LiquidAI template produced exactly the same input tokens after we removed the previous call. The lmstudio-community template included the call and used 18 more tokens. The quantity itself reached both renderings. Replacing 42 with 17 changed each token sequence. [Frozen protocol](protocol.md) · [LiquidAI template](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit/blob/ce5150fd0de58d2bba8d88cc87f13e4fbda5977a/chat_template.jinja) · [Community template](https://huggingface.co/lmstudio-community/LFM2.5-1.2B-Instruct-MLX-8bit/blob/25bd235ed869880804b720b5f2df6e378684d913/chat_template.jinja)

| Rendered input | Input tokens | LiquidAI checkpoint | Community checkpoint |
| --- | ---: | --- | --- |
| LiquidAI, 42 units | 192 | `2` | `2` |
| Community, 42 units | 210 | Archived 42-unit sentence | Same sentence |
| LiquidAI, zero-stock replacement | 192 | `0` | `0` |
| Community, zero-stock replacement | 210 | Archived zero-stock sentence | Same sentence |

Both tokenizers assigned identical IDs to each fixed rendered string. Under each checkpoint, changing the rendered prompt changed the output. Swapping checkpoints at fixed rendering preserved the answer on all four inputs. Decoding the output IDs directly reproduced every streamed answer. Both packages use the same model family name and have different weight file hashes. The archived run did not record its template digest, so we can identify a configuration that reproduces it and the input difference that changes this result. Its original installation remains unknown. [Complete answers and hashes](results.json) · [Independent repeat](verification.json)

## Check a request before inference

Open **Before inference** and load either saved preview. The LiquidAI preview flags `/messages/2/tool_calls`. The community preview shows the token sequence changed for both tested fields. The check compares the entire ordered sequence, so a substitution can register even if the token count stays the same. A model can receive a field and still disregard it. [LiquidAI preview](liquid-preview.json) · [Community preview](community-preview.json)

To measure your own request, use the tokenizer and template from your workflow. This example requires Transformers 5.23.0 and a local fast-tokenizer directory with its chat template. It does not load model weights.

```sh
study=examples/local-mlx-tool/rendering
cargo run --locked -- probe-render "$study/request.json" "$study/fields.json" /tmp/render-preview.json -- \
  python3 scripts/render_chat_tokens.py "$LFM_SNAPSHOT" --renderer-id inventory-staging
```

Select JSON pointers to fields you expect to affect the model input. NexusWeave passes the adapter a version-one object with a `requests` array. Its first entry is the original body. Each following entry removes one selected field independently. Return `renderer`, a `renderer_sha256` fingerprint for the rendering configuration, and `tokens`, one ordered array of unsigned token IDs per request. Use `null` for an input the renderer rejects. The original must render. The CLI records fingerprints and lengths, then the dashboard verifies that each condition still matches the supplied request. The adapter supplies the token evidence. For hosted or multimodal systems, use this check only on a token channel your workflow can observe.

## Reproduce the model experiment

Run the saved checks anywhere with Python. To regenerate, use the [Apple Silicon model environment](../../bfcl-transfer/mlx/README.md#reproduce) and download both pinned snapshots above. Each run verifies the local files before loading them. Choose unused output paths.

```sh
study=examples/local-mlx-tool/rendering
python3 "$study/check.py"
HF_HUB_OFFLINE=1 python "$study/study.py" --liquid "$LFM_SNAPSHOT" --community "$COMMUNITY_SNAPSHOT" \
  --output /tmp/inventory-renderings.json
HF_HUB_OFFLINE=1 python "$study/study.py" --liquid "$LFM_SNAPSHOT" --community "$COMMUNITY_SNAPSHOT" \
  --repeat /tmp/inventory-renderings.json --output /tmp/inventory-repeat.json
```
