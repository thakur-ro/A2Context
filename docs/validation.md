# Validate a deployment

Run `make check` for the public regression suite and `make check-ui` for desktop and mobile dashboard journeys. The test and demo inputs are synthetic. Run `make verify-image` to export telemetry into the shipping container, retry it and confirm the captured records survive a restart.

In staging, compare the same workload with its usual asynchronous exporter and with a second destination for A2Context. Check inference latency, processing lag, queue drops, memory and disk growth at your expected traffic level. Query `/api/sidecar/health` to see how many requests have usable context. A quiet feed can indicate missing inputs.

[Deployment procedure](../DEPLOYMENT.md) · [Test prerequisites](../CONTRIBUTING.md)
