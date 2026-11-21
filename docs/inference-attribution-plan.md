# Inference attribution

A2Context should help a developer see which context is influencing a decision as inference unfolds, how that influence changes, and which other available inputs have little measured effect or reinforce each other. Clicking a result should take the developer back to the prompt source and the inference position that the evidence covers.

The [research preview](research-preview.md) records the latest holdout, the decisions it changed, and the next experiment.

## What we can observe

| Evidence | What it supports |
| --- | --- |
| **Captured input** | The model received this instruction, tool description, retrieved passage, or prior message. An anchor leads back to the trace. |
| **Recorded link** | A declared relationship, invocation ID, or matching value connects an input to an observed action. The link gives the investigator a candidate to test. |
| **Request replay** | A specific edit changed or preserved an outcome under the tested conditions. This works with an endpoint that supports reruns. If it exposes output probabilities, the experiment can track a decision margin at a fixed prefix. |
| **Shadow proposal** | An edit to a pending request changed how often a runner returned a declared proposal. The workflow can examine the result before committing an action. [Shadow probe example](guide.md#test-context-with-shadow-proposals) |
| **Runtime observation** | An instrumented inference run exposes an activation or a routing signal at a specific layer and token position. The readout depends on the runtime and probe. |
| **Activation intervention** | Replacing a defined internal activation during a run changed a measured decision score. This supports a claim about the tested component in that setting. |

Model weights are learned parameters. Activations change with the input and the generated prefix. A large activation or attention weight is a candidate signal. Use interventions to test whether the observed component matters to the measured output. Circuit tracing provides one approach to studying internal computation and validating proposed pathways. [Circuit Tracing](https://transformer-circuits.pub/2025/attribution-graphs/methods.html), [Attention is not Explanation](https://aclanthology.org/N19-1357/)

## First experiment

Use one instrumentable open-weight model and the existing return-window scenario. The first case gives a 30-day policy and a 60-day fallback. Record the model revision, tokenizer and rendered prompt digests, captured source ranges, and the exact generation position used for scoring. Check that token positions align before swapping activations between runs. Use a fixed decision metric such as the difference in log probability between two candidate tokens.

1. Record the original run and runs after separate policy and store-name edits. Keep both edits the same length in tokens as the original and check which positions change. In a case with overlapping policy sources, test both edits together when the individual effects are small.
2. Capture a limited set of activations around the answer position. Replace selected layer and token activations with activations from the comparison run and measure the same score. Compare the result with patches at unrelated positions. [How to use and interpret activation patching](https://arxiv.org/abs/2404.15255), [TransformerLens activation capture](https://transformerlensorg.github.io/TransformerLens/generated/code/transformer_lens.model_bridge.bridge.html)
3. Show the recorded result with the tested alternatives, the selected prompt sources, and the measured effects. Keep surprising results, weak signals, and missing measurements visible. A small individual effect applies to the tested edit. A larger joint effect can reveal redundancy or interaction.

The pilot should reproduce the baseline decision, trace a result back to the exact request and activation intervention, and show how the measured changes compare with the controls. Repeat on a tool choice or routing task before treating the view as a general workflow diagnostic. Keep the experiment narrow enough to distinguish model behavior from changes introduced by the probe.

## What A2Context adds

Use the same anchors to connect inference evidence to a real workflow. When the runtime supplies deeper evidence, follow a named source through the prompt, the prediction position, and the resulting decision. When the provider exposes a narrower interface, show the captured trace and available replay measurements. ContextCite is a related method for attributing generated statements to context. [ContextCite](https://proceedings.neurips.cc/paper_files/paper/2024/hash/adbea136219b64db96a9941e4249a857-Abstract-Conference.html)

An adapter should report the model and tokenizer revisions, the input and output positions covered, the probe or intervention, the score and units, and any capture gaps. Keep comparable measurements together when looking for repeated patterns across time, threads, users, or organizations. Make the evidence level and coverage visible in both the browser and agent-facing views.

## First instrumented result

On the synthetic return-window case, the original prompt produced **30** with +4.50 log odds for the 30 token over the 60 token. Changing only the policy value produced **60** with −18.19 log odds. Renaming Alder to Cedar produced **30** with +4.63 log odds. The clean-to-clean patch and the full edited-state patch at block 8 reproduced their respective scores exactly. [Measured report](../examples/local-mlx/activation-report.json), [Pinned model revision](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit/tree/ce5150fd0de58d2bba8d88cc87f13e4fbda5977a)

Patching the edited policy token flipped the preference to 60 at blocks 0–13. At blocks 14–15, that patch left the original margin unchanged. Patching just the prediction position with the policy edit flipped the preference at blocks 12–15, with a temporary rise in preference for 30 at blocks 10–11. Patching the edited store-name positions kept the margin within 0.25 log odds of the original at every block. Block numbers are zero-based. The probe covers the output of one decoder block at a time, at one output prediction position. It supports a claim about these interventions in this case. The transfer path between positions, the contribution of individual heads, and the behavior over multiple output tokens remain open.

The report is attached to the original decision by matching the reproduced answer and the digests of the tested context entries. The browser shows it as a separate instrumented replay. This example covers one synthetic conversation. Repeating it on other decisions and runtimes is needed to learn whether any measured patterns recur across conversations. [Open the probe in A2Context](guide.md#inspect-activations-on-the-return-window-decision)

## Follow two predictions in one answer

The second fixture asks for a return window and a refund time. It uses two numeric fields after the pilot prompt produced inconsistent formatting for a contact-channel answer. The model generated 30 days and 14 days across 12 tokens. The probe measures two positions, holding the earlier generated tokens fixed to the original answer for every edit. The two original-versus-alternative contrasts are 30 versus 60 at token 4 and 14 versus 21 at token 11. Token numbers here start at 1. [Fixture and exact edits](../examples/local-mlx-timeline/case.json), [Measured report](../examples/local-mlx-timeline/activation-report.json)

| Prompt edit | Return window | Refund timing | Greedy answer |
| --- | ---: | ---: | --- |
| None | +7.63 | +15.63 | 30 days, 14 days |
| Return window from 30 to 60 | −4.13 | +3.75 | 60 days, 14 days |
| Refund time from 14 to 21 | +7.25 | −15.25 | 30 days, 21 days |
| Store name from Alder to Cedar | +7.63 | +15.50 | 30 days, 14 days |

The scores are natural log odds for the original value over the alternative at the fixed prefix. The return-window edit also lowered the refund margin by 11.88 log odds with the earlier answer still fixed to 30 days. Swapping only the return-policy token's block-0 output lowered the refund margin by 12.00. The preference for 14 survived every return-policy swap at the source, while the refund-policy swap flipped it at 14 of 16 blocks. Store-name swaps changed either margin by at most 0.25. Both clean and full-donor controls reproduced their scores exactly at both checkpoints.

At the second checkpoint, the fixed original text can disagree with an edited return policy. The result measures what the model does under that condition. The two positions use different token contrasts, so the margins have a common unit and each retains its own baseline. The intermediate tokens, alternative generation paths, and transfer routes between prompt and answer positions remain unmeasured. [Activation patching guide](https://arxiv.org/abs/2404.15255)

## Compare the measured pattern across conversations

The cohort repeats the same model, policies, and edits across six different user questions. Its synthetic ownership fields cover three conversations for one user, a second user in the same organization, and two conversations in another organization with the first user's identifier. All six runs met the token-alignment and patch-control checks. [Declared questions and ownership](../examples/local-mlx-timeline/cohort.json), [Measured reports](../examples/local-mlx-timeline/cohort/activation-reports.json)

At block 0, the return-policy source swap flipped the return-window preference in all six conversations. At the refund token it lowered the margin by 8.25 to 12.00 log odds, and flipped the preference in one conversation. The refund-policy source swap flipped the refund preference in all six. The store-name swaps moved either score by at most 0.50 log odds. Each checkpoint retains its own comparison. Four refund readouts had a baseline prefix with two spaces before the newline and two had a prefix without those spaces. The inspector groups these readouts separately. The first checkpoint shared the same earlier tokens in all six conversations.

The six runs share one model and task design. The organization and user labels exercise scoping. Cross-task and cross-model replication remain open. [Inspect the cohort](guide.md#find-repeated-activation-effects-across-conversations)

An MCP client can query the same groups within a host-selected scope, open the contributing turn, and follow the score back to the exact checkpoint or source in the inspector. [MCP query example](guide.md#query-activation-effects-through-mcp)

```sh
python3 scripts/probe_mlx_timeline_cohort.py /path/to/local-model /path/to/new-output-directory \
  --model-id LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit \
  --revision ce5150fd0de58d2bba8d88cc87f13e4fbda5977a
```

## Trace influence through a tool proposal

The synthetic routing task offers two read-only tools, `stock` and `price`, and a product ID. The model proposed `stock(product_id=42)` on the original prompt. The call syntax is normalized with the runtime's tool parser. The model repeated the tool and argument values in the text after the proposal, so both checkpoints were anchored inside the tool syntax. [Declared fixture](../examples/local-mlx-routing/case.json), [Measured report](../examples/local-mlx-routing/activation-report.json), [LFM model card and tool syntax](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct#tool-use)

| Prompt edit | Tool choice log odds | Product ID log odds | Greedy proposal |
| --- | ---: | ---: | --- |
| None | +4.25 | +26.59 | `stock(product_id=42)` |
| Request type from stock to price | −2.13 | +23.44 | `price(product_id=42)` |
| Product ID from 42 to 73 | +4.00 | −20.06 | `stock(product_id=73)` |
| Store name from Alder to Cedar | +4.38 | +26.41 | `stock(product_id=42)` |

Swapping activations at the edited request-type token flipped the tool-name preference at 10 of 16 decoder blocks. Swapping at the edited product-ID token flipped the argument preference at 14 of 16. The store-name swaps changed either score by at most 0.38 log odds and flipped neither preference. At the argument checkpoint, every run was scored with the original tool name already in the fixed prefix. The experiment records the model's proposal and stops before execution. It covers one tool-routing case on the same model as the text experiments. [Inspect the routing case](guide.md#inspect-a-tool-choice-and-its-argument)

```sh
python3 scripts/probe_mlx_answer_timeline.py /path/to/local-model \
  --case examples/local-mlx-routing/case.json \
  --model-id LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit \
  --revision ce5150fd0de58d2bba8d88cc87f13e4fbda5977a > tool-activation.json
```

## Test the report across two runtimes

We repeated the return-window and refund task with the same prompt messages and three edits in MLX and in Transformers with PyTorch. The values are single digits so both tokenizers can represent each candidate as one token. We used Rowan for the store-name edit after checking token alignment. In both free generations the original values were 3 and 4 days. The policy edits changed their corresponding values, and the store-name edit preserved the numbers. The models produced different wording and each fixed-prefix experiment used its own generated tokens. [Matched MLX case](../examples/runtime-comparison/case-mlx.json), [Matched Transformers case](../examples/runtime-comparison/case-transformers.json)

| Source swapped at block 0 | LFM in MLX | Qwen2 in Transformers |
| --- | ---: | ---: |
| Return-policy swap at the return-window token | −14.44 log odds | −17.52 log odds |
| Return-policy swap at the later refund token | −6.38 log odds | −7.21 log odds |
| Refund-policy swap at the refund token | −16.88 log odds | −21.42 log odds |

The return-policy swap weakened the later refund preference in both runs while it still favored 4 days. The store-name swap changed either metric by at most 0.25 log odds across the two runs. Both runtimes passed the clean and full-donor controls. Each model used its own architecture, weights, tokenizer, and generated prefix, so this comparison cannot isolate a runtime effect. It verifies that independently instrumented runtimes can emit the same evidence format and shows a similar pattern on this one synthetic task. [MLX measurements](../examples/runtime-comparison/activation-report-mlx.json), [Transformers measurements](../examples/runtime-comparison/activation-report-transformers.json), [PyTorch forward hooks](https://docs.pytorch.org/docs/stable/generated/torch.nn.Module.html#torch.nn.Module.register_forward_hook)

## Measure the entire recorded output

The full-output probe holds the recorded prefix fixed at every step. It subtracts the original natural log probability of the emitted token from its log probability after a source swap. Each intervention produces a score for every recorded token, including punctuation and the text following a tool proposal. This readout is related to ContextCite's use of original-response likelihood to test context changes. Here the intervention swaps a decoder block's output at the edited source tokens. [ContextCite](https://proceedings.neurips.cc/paper_files/paper/2024/file/adbea136219b64db96a9941e4249a857-Paper-Conference.pdf)

In the 22-token MLX tool output, swapping the product-ID source at block 0 reduced the likelihood of `42` inside the proposal by 20.13 log units. It reduced the likelihood of the later `42` in the explanation by 0.06 log units when the original proposal was already part of the fixed prefix. In the 16-token Qwen answer, the return-policy swap at block 0 reduced the original return value by 8.05 log units, the word “days” that followed it by 0.21, and the later refund value by 0.53. The clean and full-donor checks reproduced all token scores in both runs. [Tool output measurements](../examples/local-mlx-routing/output-token-report.json), [Text output measurements](../examples/runtime-comparison/output-token-report-transformers.json)

These results track one recorded path per example. At the second `42`, the model already has the first `42` in its prefix. The scores alone cannot identify how the influence traveled through the model or what would happen after an earlier token changed. [Inspect the full output](guide.md#inspect-every-recorded-output-token)

## Let the model continue after the swap

In the Qwen return and refund case, we fixed the recorded prefix before a selected output token and used the model to generate the rest. For the activation condition, the adapter supplied the donor's hidden values at the edited source positions on every decoding step. It kept the original input text and re-ran the full prefix with the cache disabled. The separate prompt-edit condition used the edited input text with the same fixed earlier output. [Transformers cache controls](https://huggingface.co/docs/transformers/v4.50.0/kv_cache), [Saved continuations](../examples/runtime-comparison/generation-branches-transformers.json)

| Source swap at block 0 | First token allowed to change | Recorded values | Values after the swap |
| --- | ---: | --- | --- |
| Return policy | 6 | 3 days, 4 days | 6 days, 4 days |
| Return policy | 14 | 3 days, 4 days | 3 days, 4 days |
| Refund policy | 14 | 3 days, 4 days | 3 days, 7 days |
| Store name | 6 | 3 days, 4 days | 3 days, 4 days |

All four clean-to-clean checks reproduced the full recorded token sequence. The adapter counted a swap at every generated step, including the stop token. The corresponding prompt-edit run matched the patched completion in these four cases. Starting at token 14 holds the original three-day return window fixed and lets the refund value change. The results cover one greedy model and one task with four selected interventions. [Inspect the continuations](guide.md#continue-decoding-after-a-source-swap)

## Screen captured sources by masking attention

We measured every alignable source in the recorded Qwen return-and-refund decision. The adapter mapped the captured UTF-8 request ranges to rendered prompt tokens. For each source it masked attention to those keys at every decoder block while keeping the original token positions and earlier answer tokens. One forward pass per source measured all 16 recorded output tokens. The all-visible control reproduced their scores exactly. A second run with the pinned model and runtime produced the same report. [Saved report](../examples/runtime-comparison/source-visibility-transformers.json), [Geva et al., attention interventions](https://aclanthology.org/2023.emnlp-main.751/)

| Masked source | Change at return digit `3` | Change at refund digit `4` | Largest absolute change in the answer |
| --- | ---: | ---: | --- |
| Format instruction | −0.007 | −0.001 | −0.73 at ` is` |
| Returns policy | −1.74 | −0.67 | −1.74 at `3` |
| Refund policy | −0.010 | −5.91 | −5.91 at `4` |
| Store name | +0.001 | ≈0 | +0.20 at ` is` |
| Question | −0.003 | ≈0 | −4.08 at ` return` |

Scores are changes in natural log probability, with the masked score minus the original score. The policy sources had their largest effects at the corresponding values. The question mattered more to the earlier wording, and the store name had a smaller measured effect across the response. This is a one-source-at-a-time intervention on one recorded answer. Redundant sources can be missed, and the result depends on the fixed prefix and the attention mask. ContextCite measures fixed-response likelihood across many prompt ablations. AttriBoT speeds up a leave-one-out context attribution method. A2Context attaches the measured token effects and coverage to the original workflow decision and source locations. [ContextCite](https://arxiv.org/abs/2409.00729), [AttriBoT](https://github.com/r-three/AttriBoT), [Reproduce the probe](guide.md#test-source-visibility-on-a-recorded-answer)

## Test whether two sources back each other up

We repeated the Qwen experiment with a second copy of the refund policy at a separate prompt position. We tested both copies together and used one copy paired with the store name as a control. For each pair, the four conditions were both visible, each source masked separately, and both masked together. Every condition kept the same token positions and earlier output tokens. [Synthetic input and recorded answer](../examples/runtime-comparison/paired-visibility-evidence.json), [Measured report](../examples/runtime-comparison/paired-visibility-transformers.json), [Two-factor experimental design](https://itl.nist.gov/div898/handbook/prc/section4/prc437.htm)

| Source pair at the refund digit `4` | First only | Second only | Both masked | Interaction |
| --- | ---: | ---: | ---: | ---: |
| Refund policy and its copy | +0.00018 | +0.00003 | −5.84435 | −5.84456 |
| Refund policy and store name | +0.00018 | +0.00022 | +0.00033 | −0.00006 |

The columns show changes in natural log probability. The interaction subtracts the sum of the individual changes from the joint change. Either copy was sufficient to preserve the high probability of the recorded digit under this intervention. The paired control preserved that probability at the same position. This test uses an exact duplicate, one Qwen2 answer, and a fixed output prefix. It gives a way to test a redundancy hypothesis at a particular token. Semantic duplicates, groups of three or more sources, and free generation under the joint mask remain open. [Inspect the paired masks](guide.md#test-a-pair-of-sources)

## Measure a proposed tool name and argument

We used the same Qwen2.5 checkpoint for a synthetic product lookup with two tool definitions. Greedy generation reproduced `stock(product_id=42)`. The adapter verified the parsed name and arguments against the recorded call, and the all-visible control reproduced every unmasked token score. [Recorded request and proposal](../examples/runtime-comparison/visibility-routing/evidence.json), [Measured report](../examples/runtime-comparison/visibility-routing/report.json)

| Masked source | Change at `stock` | Change across `42` |
| --- | ---: | ---: |
| System instruction | +0.00013 | −0.00001 |
| Request type | −0.26029 | 0 |
| Product ID | −0.00018 | −4.86417 |
| Store name | −0.00002 | 0 |
| Question | −0.00002 | 0 |
| Request type and product ID | −0.26217 | −4.88383 |
| Stock definition | −3.42828 | −0.00001 |
| Price definition | +0.00019 | 0 |
| System instruction and request type | −0.00534 | −0.00001 |
| System instruction and stock definition | −3.54794 | −0.00003 |
| Both tool definitions | −0.01579 | −0.00001 |

The table gives masked minus original log probability. The argument column sums the two digit scores, each conditioned on the preceding recorded tokens. Masking the stock definition alone lowered the recorded name's score more than masking the request type. Masking both definitions brought the score near the baseline, with an interaction of +3.41230 log units. The product ID had the largest measured effect on the argument.

The instruction mask includes the two template newlines that share its last token. Its largest individual effect was −0.03797 log units at the opening `<tool_call>` delimiter. The tool-name score barely moved. Masking the instruction with the request type moved the score closer to the baseline than masking the request type alone. The interaction was +0.25482 log units. These are fixed-prefix effects from one proposal. Whether the model would generate a different tool call under the same masks remains open. [Reproduce the probe](guide.md#test-source-visibility-on-a-tool-proposal)

## Compare routing effects across conversations

We reproduced seven recorded `stock` or `price` proposals on the same Qwen2.5 checkpoint. The cohort uses synthetic conversation, user, and organization identifiers. Two request bodies repeat across organizations. The north follow-up adds the prior stock request and answer to the otherwise identical price request for product 19. [Cohort cases](../examples/runtime-comparison/visibility-routing-cohort/cohort.json), [Measured reports](../examples/runtime-comparison/visibility-routing-cohort/reports.json)

| Masked source | Change when the recorded tool was `stock` | Change when the recorded tool was `price` |
| --- | ---: | ---: |
| Stock definition | −3.43540 to −3.42828 | +0.00008 to +0.19716 |
| Price definition | +0.00019 | −7.97884 to −3.13614 |

Each value is a change in log probability at the recorded tool-name token. The selected tool's definition had the largest negative effect on the name within each of the two cohorts when masked. In the follow-up, masking the prior assistant reply lowered the recorded `price` score by 1.08491 log units. The earlier output tokens stayed fixed. The ranges describe these seven measurements and the ownership labels let readers exercise the scope filters. [Inspect the cohort](guide.md#compare-measured-tool-routes)

## Let the model decode with sources masked

We used the original stock lookup to compare the recorded `stock(product_id=42)` proposal with greedy continuations under the same masks. The all-visible controls reproduced the recorded token sequence from both starting points. [Saved continuations](../examples/runtime-comparison/visibility-routing/continuations.json)

| Masked source | Resume at | Measured proposal |
| --- | --- | --- |
| Stock definition | Tool name | `price(product_id=42)` |
| Both tool definitions | Tool name | `stock(product_id="42", tool="stock")` |
| Product ID | First digit | `stock(product_id=12345)` |

Masking both definitions left the recorded `stock` token score near baseline. The continuation kept that tool and changed the argument type and fields. This shows why a score at one token can miss a change later in the output. These results describe one recorded request, two fixed prefixes, and one greedy model. The adapter parses proposals without checking their argument schemas. [Reproduce the continuations](guide.md#continue-decoding-with-sources-masked)

## Count masked outcomes across the routing cohort

We ran four continuations for each of the seven recorded proposals, with an all-visible control at each starting token. The grouped results count decisions with saved evidence for that mask and named output. [Inspect the cohort](guide.md#compare-measured-tool-routes)

| Masked source | Recorded `stock` | Recorded `price` |
| --- | --- | --- |
| Stock definition | 3 tool changes | 4 same proposals |
| Price definition | 3 same proposals | 4 tool changes |
| Both tool definitions | 3 argument changes | 3 argument changes, 1 token limit |
| Product ID | 3 argument changes | 4 argument changes |

In the two-turn follow-up, masking the current product ID changed the proposed argument from `19` to `42`, the ID present in the earlier request and reply. The same mask in the `price` request for product 19 without that history generated `12345`. The cases share a checkpoint and greedy decode settings. Their synthetic ownership labels let readers inspect the results at thread, user, and organization scope. [Saved cohort reports](../examples/runtime-comparison/visibility-routing-cohort/reports.json)
