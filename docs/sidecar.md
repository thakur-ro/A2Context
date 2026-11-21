# Run A2Context beside inference

```sh
cargo run --locked -- sidecar --db /tmp/a2context.sqlite
```

The receiver accepts OTLP/HTTP on loopback. The dashboard reads the same SQLite store. Content capture is off by default. For a redacted development stream use a separate database with `--capture-content`. Use `--retain-days` to change the seven-day retention window.

## Inspect the receiver

| Endpoint | What it returns |
| --- | --- |
| `GET /api/sidecar/health` | Accepted batches, queue drops, processing lag and capture coverage |
| `GET /api/sidecar/findings` | Grouped observations and references to a supporting call |
| `GET /api/sidecar/trace?id=…` | Calls and their available source evidence |
| `GET /api/sidecar/tool-reference-score` | A scoped handoff check, assessment coverage and supporting records |

### Fan out without changing observability platforms

One local OpenTelemetry Collector can send the same received trace stream to the sidecar and to your current backend. This sketch uses independent bounded sending queues. Use a separate redaction pipeline or equivalent filter if capture policy differs between destinations.

```yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: 127.0.0.1:4318
processors:
  batch:
    send_batch_size: 32
    send_batch_max_size: 32
exporters:
  otlphttp/a2context:
    traces_endpoint: http://127.0.0.1:8040/v1/traces
    sending_queue:
      queue_size: 128
      num_consumers: 1
  otlphttp/observability:
    traces_endpoint: ${env:OBSERVABILITY_OTLP_TRACES_ENDPOINT}
    sending_queue:
      queue_size: 128
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/a2context, otlphttp/observability]
```

For LangSmith, configure its [OTLP endpoint and headers](https://docs.langchain.com/langsmith/trace-with-opentelemetry) on the observability exporter. This fan-out sends traces to both destinations. Use the [LangSmith importer](guide.md#import-langsmith-traces) for saved runs. [Collector queue and retry behavior](https://opentelemetry.io/docs/collector/resiliency/)


[Deployment and staging checks](../DEPLOYMENT.md)
