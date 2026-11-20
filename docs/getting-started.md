# Connect your first workflow

You’ll start a live receiver, send it a known trace batch, then switch to your application’s exporter. Run the commands from the repository root with Rust and curl. [Try the recorded demo first](../README.md#try-the-demo)

## 1. Start an empty workspace

In one terminal, run:

```sh
cargo run --locked -- sidecar --db first-workflow.sqlite
```

Open the printed address, normally `http://127.0.0.1:8040`. Leave this process running. The database persists between runs. The default capture policy retains trace metadata and drops prompt and response content.

## 2. Verify that a trace arrives

In a second terminal, send the same sample that powers the recorded demo:

```sh
curl --fail-with-body --header 'Content-Type: application/json' \
  --data-binary @examples/exchange-approval/exchange.otlp.json \
  http://127.0.0.1:8040/v1/traces
curl --fail http://127.0.0.1:8040/api/sidecar/health
```

On a fresh database, look for `coverage.traces` and `coverage.inference_spans` both equal to **3**, and `content_capture` equal to **false**. Reload the dashboard. **Telemetry received** confirms that the batch arrived and shows the counts. This run discarded the source declarations and replay evidence, so the feed has no recommendations. You can resend the sample to test delivery again. Its trace IDs stay the same.

## 3. Send a span from your application

If your application already exports OpenTelemetry traces, use its existing batch exporter. For a local SDK that reads the standard environment variables, set these in the application's environment before starting it:

```sh
export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:8040/v1/traces
```

On the span around a chat request, set `gen_ai.operation.name` to `chat` and `gen_ai.request.model` to your model identifier. A stable `gen_ai.conversation.id` groups turns in a thread. Run one inference, open **Threads**, and find your conversation. If your exporter already feeds LangSmith or another backend, add NexusWeave as a second destination in the Collector. [Collector fan-out](sidecar.md#fan-out-without-changing-observability-platforms) · [OpenTelemetry exporter settings](https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/)

**Starting without an instrumented app?** This Python example sends a tool choice, an inventory lookup and an answer through the real SDK. It uses synthetic data and makes no model calls. With the receiver from step 1 running, try:

```sh
python3 -m venv /tmp/nexusweave-otel
/tmp/nexusweave-otel/bin/pip install opentelemetry-exporter-otlp-proto-http==1.45.2
/tmp/nexusweave-otel/bin/python examples/send_otlp_inventory.py --conversation-id first-inventory
```

Look for `first-inventory` in **Threads**. After one run, the counts from step 2 increase to **4 traces** and **5 inference spans**. [Inventory producer](../examples/send_otlp_inventory.py)

## 4. See a source-level finding

Stop the receiver with Ctrl+C. For this synthetic sample, start another workspace with content capture enabled:

```sh
cargo run --locked -- sidecar --db first-findings.sqlite --capture-content
```

Repeat the curl commands in step 2 and reload the dashboard. On a fresh database, you’ll see **2 recommendations**. Choose **Which instructions disagreed?** The cache supplied `approved` and the customer supplied `hold`. The second recommendation links to a saved comparison of the same proposal under two approval policies.

For your own conflict finding, instrument the request builder with stable source IDs, source locations and the rule key, scope and value each source declares. Send a suitably redacted stream to the capture-enabled workspace. You can check the declarations on a pending request before sending any telemetry or calling a model. [Working request with two declarations](../examples/exchange-approval/pending.json) · [Check context before inference](guide.md#check-context-before-inference) · [Deploy the sidecar](../DEPLOYMENT.md)

### If the dashboard is still empty

| What you see | What to check |
| --- | --- |
| curl cannot connect | Use the address printed by the live sidecar. In a container, loopback belongs to that container’s network namespace. |
| The sample arrives, but your app’s trace does not | Check that an exporter is installed, that it uses OTLP over HTTP, and that the trace endpoint includes `/v1/traces`. Flush the exporter before a short-lived process exits. |
| Inference spans arrive, but no rule finding appears | Check `content_capture` and `coverage.with_rules` in health. Use a fresh database when changing the capture policy. Confirm the same rule key and scope appear on both sources. |
