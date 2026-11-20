# Contributing to NexusWeave

Use stable Rust with `rustfmt` and Clippy, Python 3.11 or later, Node.js 24 and Make. The Rust binary embeds the dashboard. Run `make demo` and open the address it prints. [Watch the recording](docs/demo.md)

## Find the code

| Path | Responsibility |
| --- | --- |
| [src/sidecar](src/sidecar) | OTLP listener, bounded worker, SQLite storage, recommendation queries and the HTTP dashboard |
| [src/replay](src/replay) | Request-bound source edits and the checks for candidate scores, independent trials and comparable outcomes |
| [ui](ui) | Dashboard and drill-down views served by the binary |
| [schema](schema) | Versioned data contracts for workflow and model adapters |
| [examples](examples) and [tests](tests) | Runnable integrations, saved studies and regression fixtures |
| [Dockerfile](Dockerfile) and [deploy](deploy) | Shipping image and example pod integration |

## Check a change

```sh
make check
make verify-image
```

The first command checks formatting, lints, Rust tests, JavaScript and end-to-end findings on saved workflows. The second requires Docker and exercises the actual runtime image. To include the independent Protobuf/gzip exporter, run the SDK test in the [validation guide](docs/validation.md). To check your deployment's volume, capture policy, queue and latency budgets, follow the [staging procedure](DEPLOYMENT.md#before-sending-production-traffic).

After editing `ui/mcp-app/view.js` or `ui/mcp-app/view.html`, run `npm ci --prefix ui/mcp-app` and `npm run build --prefix ui/mcp-app`. The build refreshes the HTML embedded in the Rust binary and its bundled dependency licenses. Run `npm run check --prefix ui/mcp-app` to verify both generated files before submitting changes.

## Send a change

Describe the workflow you were inspecting and the expected result. For an importer, include a minimal synthetic input and show which recorded fields it maps to. For a UI change, include the view and a way to reproduce it with a fixture. For a replay change, include the intervention and the outcome the runner scored.

Use synthetic fixtures for reproductions. Keep raw traces, captured requests, credentials, and private replay outputs in the ignored `data/` directory. If an example draws on published data or research, record the source and license in [CREDITS.md](CREDITS.md).

See the [evidence schema](schema/evidence-1.json) for the interchange format and the [guide](docs/guide.md) for the import and replay workflows.
