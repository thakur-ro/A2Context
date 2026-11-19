# Contributing to NexusWeave

Run `cargo run --locked -- serve fixtures/support.json` from the repository root and open the local inspector. The fixture exercises conflicts, duplicate rules, and repeated findings across synthetic organizations. [Open the inspector](http://127.0.0.1:8040)

## Run the checks

Use stable Rust with `rustfmt` and `clippy`, and Node.js 24. The browser UI is served by the Rust binary and has no separate build step.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
(for source in ui/*.js; do node --check "$source" || exit 1; done)
node --test tests/*.test.mjs
```

After editing `ui/mcp-app/view.js` or `ui/mcp-app/view.html`, run `npm ci --prefix ui/mcp-app` and `npm run build --prefix ui/mcp-app`. The build refreshes the HTML embedded in the Rust binary and its bundled dependency licenses. Run `npm run check --prefix ui/mcp-app` to verify both generated files before submitting changes.

## Send a change

Describe the workflow you were inspecting and the expected result. For an importer, include a minimal synthetic input and show which recorded fields it maps to. For a UI change, include the view and a way to reproduce it with a fixture. For a replay change, include the intervention and the outcome the runner scored.

Use synthetic fixtures for reproductions. Keep raw traces, captured requests, credentials, and private replay outputs in the ignored `data/` directory. If an example draws on published data or research, record the source and license in [CREDITS.md](CREDITS.md).

See the [evidence schema](schema/evidence-1.json) for the interchange format and the [guide](docs/guide.md) for the import and replay workflows.
