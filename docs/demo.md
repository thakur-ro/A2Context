# When the customer stops the second exchange

[![Recorded investigation of a held exchange, the conflicting instructions and the resulting order state](media/exchange-approval.gif)](media/exchange-approval.mp4)

A support agent verifies the customer, looks up two orders and a replacement product, and requests the first exchange. Before the second write, the customer asks it to leave the puzzle order alone. The agent receives both that message and a cached confirmation that still authorizes both exchanges.

## Run the investigation

From the checkout with stable Rust:

```sh
cargo run --locked -- demo
```

Open the address it prints. The demo loads saved telemetry and measurements into a temporary SQLite database. It requires no API key.

From the workspace homepage, type **What did verify_customer return?** to find the tool result without knowing the conversation name. With a model connected, ask **Was the shopper’s identity confirmed?** The recording uses a local model for that question. Type **Show call** and the number next to the result to read the supporting record. [Connect a model](chat-canvas.md#connect-a-model)

1. The welcome page shows workspace totals and recurring themes. Choose **Exchange agent** for its stats and recommended investigation. Ask **Which instructions disagreed?** to compare the cached approval with the customer’s hold.
2. Type **What did the customer ask for, and did the approval check change the outcome?** Read the customer’s hold and the two sandbox outcomes together. Without the check, the puzzle order changed to `exchange_requested`. With the check, it stayed `delivered`.
3. Type **Where else did that repeat?** Two conversations contributed to this finding. Read an example in the interview, or ask **Show the order states** to see that the earlier boots exchange stayed in place under both conditions.

To follow the conversation directly, type **Read conversation “Wait on the second exchange”**. Ask **What happened before call 2?** to see the verification result, then **What happened after that call?** to return to the lookup. **Show call 2** opens its inputs and results in place.

From **All workflows**, ask **Which words were sensitive to the refund policy in Returns demo?** The saved experiment opens at position 14. Ask **What about position 6?** to see which edit changed the earlier number. The chart compares probabilities at fixed output prefixes. [What the measurements support](research-preview.md#follow-a-source-through-a-workflow)

The recording uses **Workspace evidence**. The source comparison reads the two declarations, and the dispatch comparison reads the sandbox receipt. The exchange agent’s saved proposals came from Qwen. You can run the walkthrough without installing a model. [Connect a model](chat-canvas.md)

[MP4 for offline viewing](media/exchange-approval.mp4) · [Saved proposals and order states](../examples/exchange-approval/results.json)

### Diagnose missing context capture

After building the binary, run the same exchange workflow with a simulated instrumentation regression.

```sh
python3 examples/exchange-approval/demo.py --capture-gap
```

Open the printed address and ask **Show the context map**. The two resumed hold requests still contain the customer's instruction, but the workflow has lost its named fragment in both exports. The third request retains the customer anchor and satisfies the requirement. Select **Exchange authorization** on the map, then follow the hollow customer-message node to the exact request. Comparing its message with its source inventory tells the engineer where to repair capture.

## What the test covers

The public τ²-Bench trajectory supplied the two orders, catalog result, payment method and original confirmation. Both exchanges were authorized in that trajectory. We added two alternative customer messages that put the second order on hold after the first exchange request. A2Context replays the next proposal on a compact reconstruction of that checkpoint. The separate sandbox evaluates the resulting database state. [Pinned trace data](https://huggingface.co/datasets/Exgentic/agent-llm-traces/blob/70036b93a04e61b0ea2706a68b962f4f26774587/README.md) · [Scenario and checkpoint IDs](../examples/exchange-approval/scenario.json)

| Condition | Unwanted writes in the two hold cases |
| --- | --- |
| Original cache and unchecked dispatch | 2 of 2 |
| Refreshed cache and unchecked dispatch | 2 of 2 |
| Same refreshed-cache proposals and a check against current approval | 0 of 2 |

The authorization check also accepts a known valid approved proposal and rejects a mismatched item. In the approved model control, Qwen produced text without a usable tool call. The database check covers authorization and order state for this workflow. Agent task success would also require a valid proposal on authorized requests and accurate communication to the customer. [State assertions](../tests/check_exchange_demo.py) · [τ²-Bench evaluation](https://github.com/sierra-research/tau2-bench/blob/main/docs/evaluation.md)

The example attaches a `sandbox_dispatch` receipt to the shadow report. The sidecar matches the exact proposal and edited request, checks the checkpoint digest for both arms, and compares the reported state fields. A blocked run must preserve the entire supplied state projection. The sandbox runner supplies the execution evidence. [Receipt format](../schema/sandbox-dispatch-1.json)

The outcome list counts each identified conversation once within the selected source recommendation and scope. Comparisons share a guard ID, sandbox runner ID, tool, target field and projected state fields. The expected value belongs to each comparison, so a hold and an authorized write can be judged under the same design. Reports that disagree within one conversation open under **Mixed results in one conversation**, with a link for each observed outcome. A conversation without an applicable receipt remains visible as **No comparison with these settings**.

From **Conversations with findings**, open **Conversations with a sandbox dispatch comparison** to group the measured execution outcomes by source and dispatch setup. Select an outcome to see its conversations and open the exact saved comparison. The overview counts each tested conversation once, even when it appears under several setups.

## Check the next exchange before inference

With the demo running, select **Before inference**, then **Check the next exchange**. This request has the same disagreement about the puzzle order and a new customer message asking to pause it. The preview shows the conflict in the current inputs and two distinct earlier requests where the authorization check preserved sandbox state. Open the result to inspect the earlier proposal and the state comparison. Running the preview makes zero model calls. [Pending request](../examples/exchange-approval/pending.json)

To use this lookup in a workflow, provide `service.name` and `a2context.organization.id` alongside the pending request. The lookup matches the producer, exact user scope, model, request layout, source revisions, and declared rule values. It excludes the current request body, counts a repeated body once per comparison design, and surfaces disagreements as mixed. The bounded lookup uses at most 128 matching observations. Use a fresh replay to verify the pending request and its outcome.

## See what the model did on the pending request

Choose **Inspect saved tool-name scores and proposals**. This request was held out of the two earlier conversations. We scored two tool-name continuations from the same supplied Qwen tool-call prefix, then ran one free greedy generation per condition. Both edits keep the customer's hold in effect. The score is conditional on that prefix. The generated proposal and its arguments come from the separate free run. [Saved measurements](../examples/exchange-approval/pending-measurement.json)

| Request | Probability of the exchange continuation at the prefix | Free proposal |
| --- | ---: | --- |
| Original | 97.8% | Exchange with the expected arguments |
| Refresh the confirmation cache | 97.5% | Same exchange and arguments |
| Restate the customer's hold | 98.4% | Same tool, with the order ID repeated in both item fields |
| Make both edits | 98.6% | Same tool, with the same invalid item arguments |

Tool-call parsing is part of this replay. An extra pair of braces in the tagged payload is accepted by the example’s Qwen adapter. A follow-up on eight synthetic exchange requests replays both parser policies against the same raw generations and SQLite checkpoint. [Compare the parsers and dispatch states](../examples/exchange-approval/dispatch-study/README.md#how-much-did-the-tool-parser-matter)

The decoder-block chart probes the first tool-name token. At each of 24 blocks, we copied one activation vector from the edited run to the original, either at the end of the user message or at the end of the supplied tool-call prefix. The largest shift at the prefix was −0.55 log odds for the cache edit at block 22 and +0.74 for the customer-message edit at block 21. No swap reversed the preference between `exchange` and `message`, and self-swaps preserved the baseline score. The probe measures the effect of each tested swap on this score. The generated argument changes are shown separately. [Activation patching guide](https://arxiv.org/abs/2404.15255v1)

The refreshed-cache proposal still had valid arguments, so we used it for a second test. The pending request records a hold. When the sandbox receives a renewed approval after generation, checking the request's snapshot blocks the now-authorized write. Checking current approval blocks the write while the hold is in effect and allows it once approved. Both tests preserve the earlier boots exchange. The sandbox uses supplied approval values for these controls. It does not test concurrent writes or infer whether a customer message conveyed consent. [Experimental controls](../examples/exchange-approval/pending-measurement.json)

Reproduce the scores, proposals, and dispatch controls with the same pinned checkpoint and Python environment:

```sh
cargo build --locked
python3 examples/exchange-approval/measure_pending.py "$CHECKPOINT_DIR" /tmp/measured-exchange.json
cmp examples/exchange-approval/pending-measurement.json /tmp/measured-exchange.json
```

For a fresh measurement, install Transformers 4.57.6 and PyTorch 2.11.0 and supply the pinned Qwen2.5-0.5B-Instruct checkpoint. The run uses CPU float32 and greedy decoding. [Checkpoint](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/tree/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2)

```sh
cargo build --locked
python3 examples/exchange-approval/demo.py --measure "$CHECKPOINT_DIR" --output /tmp/exchange-replay.json
cmp examples/exchange-approval/results.json /tmp/exchange-replay.json
python3 tests/check_exchange_demo.py target/debug/a2context
```

To record the running dashboard again, use `node scripts/record_demo.mjs SIDECAR_URL /tmp/exchange-approval.mp4`. Pass the gateway URL and append the configured model ID to record real partner responses. The recorder follows the live UI in Chromium and encodes a 2560 × 1440 H.264 MP4 at 30 fps. It draws a 16 × 23 CSS-pixel pointer at the actual click coordinates. Set `PLAYWRIGHT_MODULE`, `CHROME_BINARY`, or `FFMPEG` if you installed those tools outside their usual locations.
