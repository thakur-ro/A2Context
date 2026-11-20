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

The first command checks formatting, lints, Rust tests, JavaScript and end-to-end findings on saved workflows. It also verifies the saved QA experiments. `make check-qa` runs those comparisons alone with Python and no model download. The second command requires Docker and exercises the actual runtime image. [Exporter validation](docs/validation.md) · [Staging procedure](DEPLOYMENT.md#before-sending-production-traffic)

For a dashboard change, run the same desktop and mobile journeys as CI. They start disposable sidecars, follow the findings and context map to recorded evidence, and check the A2UI demo. Run these commands from the repository root with Node and npm installed.

```sh
browser_tools=$(mktemp -d)
npm install --prefix "$browser_tools" --no-audit --no-fund playwright-core@1.57.0
"$browser_tools/node_modules/.bin/playwright-core" install chromium
PLAYWRIGHT_MODULE="$browser_tools/node_modules/playwright-core/index.mjs" make check-ui
```

After editing `ui/mcp-app/view.js` or `ui/mcp-app/view.html`, run `npm ci --prefix ui/mcp-app` and `npm run build --prefix ui/mcp-app`. The build refreshes the HTML embedded in the Rust binary and its bundled dependency licenses. Run `npm run check --prefix ui/mcp-app` to verify both generated files before submitting changes.

The dashboard uses A2UI 1.0 for recommendation surfaces. Its official Lit renderer and Basic Catalog are bundled into the Rust binary. After editing `ui/a2ui/surface.js`, run `npm ci --prefix ui/a2ui` and `npm run build --prefix ui/a2ui`. Run `npm run check --prefix ui/a2ui` to verify the bundle and dependency licenses. [A2UI client integration](https://a2ui.org/guides/client-integration/)

### Check a release candidate on your machine

Use a clean checkout, the browser setup above, and a running Docker daemon. Install the OTLP/HTTP exporter in a separate environment so the sidecar tests can send real Protobuf telemetry as well as JSON.

```sh
sdk_env=$(mktemp -d)
python3 -m venv "$sdk_env"
"$sdk_env/bin/python" -m pip install opentelemetry-exporter-otlp-proto-http==1.45.2
PLAYWRIGHT_MODULE="$browser_tools/node_modules/playwright-core/index.mjs" \
  make check-release SDK_PYTHON="$sdk_env/bin/python"
```

This runs the sidecar integration suite, saved study comparisons, browser journeys, both embedded UI bundle checks, container replacement, Rust API docs and installable crate verification. CI uses the same sidecar, browser, experiment and bundle targets. A local pass covers the host and container you ran it on. [Release and deployment measurements](docs/validation.md)

## Send a change

Describe the workflow you were inspecting and the expected result. For an importer, include a minimal synthetic input and show which recorded fields it maps to. For a UI change, include the view and a way to reproduce it with a fixture. For a replay change, include the intervention and the outcome the runner scored.

Use synthetic fixtures for reproductions. Keep raw traces, captured requests, credentials, and private replay outputs in the ignored `data/` directory. If an example draws on published data or research, record the source and license in [CREDITS.md](CREDITS.md).

See the [evidence schema](schema/evidence-1.json) for the interchange format and the [guide](docs/guide.md) for the import and replay workflows.
