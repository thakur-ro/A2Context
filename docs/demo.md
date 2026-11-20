# When the customer stops the second exchange

https://github.com/user-attachments/assets/5553d284-dadc-41a9-9c4f-712b637319ed

A support agent verifies the customer, looks up two orders and a replacement product, and requests the first exchange. Before the second write, the customer asks it to leave the puzzle order alone. The agent receives both that message and a cached confirmation that still authorizes both exchanges.

## Run the investigation

From the checkout with stable Rust, Make, and Python 3.11 or later:

```sh
make demo
```

Open the address it prints. The demo loads saved telemetry and measurements into a temporary SQLite database. It requires no API key.

1. Open **See what changed at dispatch**, then **Review recommendation**. The state table compares the two orders before dispatch and after the same tool proposal runs with and without the authorization check. Under **Which runs reached the expected state?**, open **Only with the check** to see both conversations and their exact replay links.
2. Open **Compare the matching proposals**. The original and refreshed-cache requests both produced `exchange_delivered_order_items` for the puzzle order. **Underlying evidence** opens the span tree and the arguments.
3. Return to the overview and open the conflicting rules for **confirmation-cache** and **latest-customer-message**. The cache still says approved and the customer’s latest message says hold.

[MP4 for offline viewing](media/exchange-approval.mp4) · [Saved proposals and order states](../examples/exchange-approval/results.json)

## What the test covers

The public τ²-Bench trajectory supplied the two orders, catalog result, payment method and original confirmation. Both exchanges were authorized in that trajectory. We added two alternative customer messages that put the second order on hold after the first exchange request. NexusWeave replays the next proposal on a compact reconstruction of that checkpoint. The separate sandbox evaluates the resulting database state. [Pinned trace data](https://huggingface.co/datasets/Exgentic/agent-llm-traces/blob/70036b93a04e61b0ea2706a68b962f4f26774587/README.md) · [Scenario and checkpoint IDs](../examples/exchange-approval/scenario.json)

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

To use this lookup in a workflow, provide `service.name` and `nexusweave.organization.id` alongside the pending request. The lookup matches the producer, exact user scope, model, request layout, source revisions, and declared rule values. It excludes the current request body, counts a repeated body once per comparison design, and surfaces disagreements as mixed. The bounded lookup uses at most 128 matching observations. Use a fresh replay to verify the pending request and its outcome.

For a fresh measurement, install Transformers 4.57.6 and PyTorch 2.11.0 and supply the pinned Qwen2.5-0.5B-Instruct checkpoint. The run uses CPU float32 and greedy decoding. [Checkpoint](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/tree/eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2)

```sh
cargo build --locked
python3 examples/exchange-approval/demo.py --measure "$CHECKPOINT_DIR" --output /tmp/exchange-replay.json
cmp examples/exchange-approval/results.json /tmp/exchange-replay.json
python3 tests/check_exchange_demo.py target/debug/nexusweave
```

To record the running dashboard again, use `node scripts/record_demo.mjs SIDECAR_URL /tmp/exchange-approval.mp4`. The recorder follows the live UI in Chromium, marks the pointer and clicks, and encodes a 1920 × 1080 H.264 MP4 with FFmpeg. Set `PLAYWRIGHT_MODULE`, `CHROME_BINARY`, or `FFMPEG` if you installed those tools outside their usual locations.
