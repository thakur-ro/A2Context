# Rust package guide

The `a2context` crate exposes normalized runs, importers, source declarations, preflight checks, replay and activation reports. Use the CLI to try a fixture before adding an adapter.

```sh
cargo doc --locked --no-deps --open
cargo run --locked -- preflight examples/exchange-approval/pending.json
```

The crate documentation includes examples checked by Rust's documentation tests. [JSON contracts](../schema) · [Workflow integration](getting-started.md)
