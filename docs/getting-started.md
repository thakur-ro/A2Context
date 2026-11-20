# Connect your first workflow

You can check one request before inference or run a receiver that finds patterns across conversations. Both paths use the exchange workflow from the demo. Start in the checkout with stable Rust and your platform’s C build tools. If the demo is still running, press Ctrl+C in its terminal. [Run the recorded demo](../README.md#try-the-demo)

## Send the sample to a running sidecar

From the checkout, start a receiver in a new directory or choose an unused database filename. The sample contains synthetic customer inputs, so you can enable content capture to see the conflicting source text.

```sh
cargo run --locked -- sidecar --db exchange-sample.sqlite --capture-content
```

In a second terminal, from the checkout, send the included OpenTelemetry batch with curl.

```sh
curl --fail --show-error -H 'Content-Type: application/json' \
  --data-binary @examples/exchange-approval/exchange.otlp.json \
  http://127.0.0.1:8040/v1/traces
```

Open the dashboard address printed by the receiver and reload after the batch arrives. You should see **2 recommendations**. Choose **Which instructions disagreed?** to compare the cached approval with the customer’s hold. Sending the same batch again reuses its trace and span IDs. To see the finding recur on a fresh conversation ID, use the [Python exporter walkthrough](#get-a-recommendation-from-a-live-span) with its own database. Stop the sample receiver with Ctrl+C before starting another on the same port.

## Check one request before inference

```sh
cargo run --locked -- preflight examples/exchange-approval/pending.json
```

Look for `exchange_authorization` in the JSON report. The cache declares `approved` and the customer declares `hold` for the same order. From a new sidecar dashboard you can open **Inspect a sample exchange request** to see the same check. Choose **Check another request** to load your own file. This check reads the assembled inputs and makes zero model calls.

To try your own request, pass the JSON your builder will send as `nexusweave.request.body`. For each source, supply its ID, version and location in that body. For a declared-rule check, give both sources the same rule key and scope and the value each supplied. Pipe the resulting versioned request to `cargo run --locked -- preflight -`. A conflict tells you the inputs disagree. A controlled edit and a measured outcome can tell you whether changing one affected the answer. [Example request](../examples/exchange-approval/pending.json) · [Source declarations](guide.md#declare-context-sources-and-decision-scope)

## Get a recommendation from a live span

This walkthrough sends synthetic approval data through a Python OpenTelemetry batch exporter. It uses the same protocol as a deployed workflow and requires no model or API key.

**Terminal 1.** Start a staging workspace that can retain the sample’s source evidence. Use a new database for the counts below.

```sh
cargo run --locked -- sidecar --db first-findings.sqlite --capture-content
```

**Terminal 2.** From the same checkout, install the exporter in a temporary environment. These commands use a POSIX shell and Python 3.11 or newer. If you already have a configured exporter, skip to [instrument your application](#instrument-your-application).

```sh
sdk_env=$(mktemp -d)
python3 -m venv "$sdk_env"
"$sdk_env/bin/python" -m pip install opentelemetry-exporter-otlp-proto-http==1.45.2
export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:8040/v1/traces
"$sdk_env/bin/python" examples/exchange-approval/first_span.py exchange-101
```

Open the address printed in Terminal 1. The **Recommendations** page now has **one conflict in one conversation**. Choose **Which instructions disagreed?** to see the cache’s approval and the customer’s hold.

Send the same source values in a different conversation, then reload the dashboard.

```sh
"$sdk_env/bin/python" examples/exchange-approval/first_span.py exchange-102
```

The same finding now covers **two conversations**. The source IDs and versions stayed the same, and the exporter assigned a fresh trace ID. [Read the complete sender](../examples/exchange-approval/first_span.py)

## Instrument your application

Leave the staging receiver running if you completed the walkthrough. For a first connection from an existing app, start a metadata-only receiver from the checkout.

```sh
cargo run --locked -- sidecar --db first-workflow.sqlite
```

1. **Connect your exporter.** For an existing batch exporter that reads standard OpenTelemetry settings, start your application with the following environment. If it already sends to LangSmith or another backend, add a second destination at the Collector to keep both. [Collector fan-out](sidecar.md#fan-out-without-changing-observability-platforms)

   ```sh
   export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf
   export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:8040/v1/traces
   ```

2. **Identify the call.** On the model span, set `gen_ai.operation.name` (for example, `chat`), `gen_ai.request.model` and a stable `gen_ai.conversation.id`. Send one staging request, then check `coverage.inference_spans` at `/api/sidecar/health`.
3. **Name the inputs you want to compare.** For this step, use the capture-enabled staging receiver above with a separate database and redacted inputs. At request assembly, record the messages and attach `nexusweave.context.semantics` to that span. In the example, `approval_inputs` takes the two structured values the workflow already has and anchors each declaration to the message part it generated. Replace those inputs with yours. Check that `coverage.with_rules` increases and open the resulting recommendation when the two known values disagree. [Working instrumentation](../examples/exchange-approval/first_span.py) · [Adapter reference](guide.md#declare-context-sources-and-decision-scope)

In metadata-only mode, **Telemetry received** confirms delivery and links to **Threads**. To check the container, storage and exporter before shipping, run the deployment checks. [Verify the shipping image](../DEPLOYMENT.md#check-the-shipping-image-locally)

### If the dashboard is empty

| What you see | What to check |
| --- | --- |
| **Connect your workflow** after sending a span | The endpoint must include `/v1/traces`. Check the exporter’s error output. Flush or shut down the exporter before a short-lived process exits. In a container, loopback belongs to that container’s network namespace. |
| **Telemetry received**, zero inference spans | Set `gen_ai.operation.name` on the model span. |
| Inference spans arrive, no conflict appears | Check `content_capture` and `coverage.with_rules` in receiver health. Use a fresh database when switching capture policy. Compare the two source anchors, rule keys, scopes and values. |
