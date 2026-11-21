# Ship A2Context with a workflow

Run the Rust sidecar alongside the process that sends the model request. The workflow serves the answer and exports spans in a background batch. A2Context receives OTLP/HTTP on loopback, processes admitted batches on a bounded worker, and commits the evidence to SQLite. The same process serves the dashboard. Model weights, Python, Node, and a model API key are unnecessary in the runtime container. [Sidecar architecture and request paths](docs/architecture.md#sidecar-architecture)

## Check the shipping image locally

For a first connection from a laptop, send a staging span and open its conflict recommendation. [Connect your first workflow](docs/getting-started.md)

From the checkout, run `make verify-image` with Make, Docker and Python 3.11 or later. This builds the repository Dockerfile. A second container shares the sidecar's network namespace, as two containers in a pod would. It sends the same synthetic inference span twice, checks that content capture is off, replaces the sidecar container on the same volume, and checks that exactly one span survived. With capture enabled on a separate synthetic database, the check also verifies that a declared conflict links back to the exported trace. A passing run ends with `PASS shipping image`. A failed assertion or command exits nonzero. The test creates and removes its own containers and volume.

To test an image you have already built, run `python3 scripts/check_image.py --image YOUR_IMAGE`. To run the source and detector checks, use `make check`. [Contributor setup](CONTRIBUTING.md) · [Exporter interoperability and load tests](docs/validation.md)

## Connect the application

Place both processes in the same network namespace. On the application’s existing OpenTelemetry batch exporter, set the trace protocol to `http/protobuf` and the traces endpoint to `http://127.0.0.1:8040/v1/traces`. If you already send to LangSmith or another backend, fan out at your Collector so both destinations receive the intended telemetry. Send stable conversation and source IDs. Content capture is off by default. Enable it explicitly on a suitably redacted stream to compare source text and trial outputs. [Collector configuration and capture settings](docs/sidecar.md#fan-out-without-changing-observability-platforms)

## Put it in a pod

The deployable artifact is the final stage of the root Dockerfile. Build it for the cluster's architecture and push it to your own registry. The example patch adds one container and its state volume to an existing single-replica Deployment. Substitute your image digest and a bound claim writable by UID 10001. Give each sidecar its own database and volume. For multiple replicas, use per-pod storage or a separately scoped deployment. Conversation counts cover the evidence received by that instance.

Review the [example Deployment patch](deploy/sidecar.patch.yaml) against your cluster without applying it:

```sh
kubectl patch deployment YOUR_WORKFLOW --type strategic \
  --patch-file deploy/sidecar.patch.yaml --dry-run=server -o yaml
```

The dashboard and ingest endpoint share one loopback listener. Operators can use `kubectl port-forward pod/YOUR_WORKFLOW_POD 8040:8040` and then open `http://127.0.0.1:8040`. Run one instance per trust boundary. Retention defaults to seven days.

## Before sending production traffic

In staging, send a representative request through **your actual exporter**. Query `GET /api/sidecar/health` from the shared namespace. Check that accepted batches and inference coverage increase, expected sources are present, redaction matches your policy, and queue drops stay within your telemetry budget. Restart the pod and confirm the expected evidence survives. Run the same workflow at representative concurrency with and without the sidecar destination. Compare inference p95, exporter retries, queue drops, processing lag, memory and disk growth against your service budgets. One CPU and 512 MiB is the repository's test configuration. Size retention and resources using your workload. [Load-test procedure and measured limits](docs/validation.md)
