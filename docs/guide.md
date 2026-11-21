# A2Context guide

## Check the advertised tools before inference

Use `a2context preflight` with an assembled request. Capture named sources at the point where the application builds the model input. Link each declaration to its actual JSON pointer and, for a substring, its UTF-8 byte range.

## Import LangSmith traces

Export runs as JSON or JSONL, then normalize them into a file the CLI can inspect. Try the command with the synthetic fixture first.

```sh
cargo run --locked -- import-langsmith fixtures/langsmith.json /tmp/returns-evidence.json
```

## Try a decision replay

From the repository root, run the synthetic returns fixture with its deterministic runner.

```sh
cargo run --locked -- replay fixtures/support.json fixtures/replay-case.json /tmp/returns-report.json -- python3 scripts/replay_synthetic.py
```

Inspect the baseline and the variants. Each trial records which declared outcome the runner produced. Request and source identity are needed to compare effects across runs.

[Connect a workflow](getting-started.md) · [Rust API](rust-api.md) · [Data contracts](../schema)
