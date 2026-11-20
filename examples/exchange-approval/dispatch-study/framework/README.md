# Where did the exchange proposal stop?

Qwen-Agent recognized 30 tool calls in the 40 saved assistant responses. **Only eight reached SQLite.** Its parser recovered a tool name from each of the 22 responses that contained an extra pair of braces. The argument string it passed to the registered tool still ended with an extra closing brace. The tool's Qwen-Agent argument validator rejected those 22 strings. Ten responses were ordinary assistant text. [Per-response parser output and order states](results.json)

| Stage | Observations among 40 saved responses |
| --- | ---: |
| Parser emitted a call for the exchange tool | 30 |
| Registered tool callback rejected the arguments | 22 |
| Registered tool executed the valid exchange | 8 |
| Assistant returned text | 10 |
| Truncations or unknown tool names | 0 |

Qwen-Agent and the NexusWeave adapter produced different pending-order states on 22 of 40 ungated replays and 12 of 40 gated replays. Under Qwen-Agent, all eight valid writes concerned approved orders, so its totals were eight writes and 32 unchanged orders with either gate policy. The first-ranked source edit changed the outcome on four requests, and the first-listed edit changed one, under both gate settings. Those four were the **approved** requests. In the original adapter's ungated test, the four were **held** requests. The aggregate count alone hides which customers were affected. [Original adapter study](../README.md) · [Strict parsing comparison](../compare_parsers.py)

## What ran

The saved `comparison` table pairs each request with its expected order state. Editing the first-ranked source restored two approved exchanges and prevented two other approved exchanges from being written. The other four requests retained their original states. [Paired outcomes under both gate settings](results.json)

The harness uses the Qwen team's published Qwen-Agent 0.0.34. The wheel published by PyPI has SHA-256 `195264f9c1880f60f23781805d69bb3ffadfe944b7b70358f7115a0dd4bc59b7`. The replay calls the public `FnCallAgent.run` entry point. A replacement model transport supplies the exact saved raw assistant string in one chunk. The framework's inherited chat implementation invokes its native Nous parser and routes the resulting call to the registered tool. The callback invokes Qwen-Agent's standard argument validator and passes valid arguments to the study's isolated two-order SQLite sandbox. Execution stops after the first tool result. The input contains no pre-parsed function call. [Published package](https://pypi.org/project/qwen-agent/0.0.34/) · [Qwen-Agent repository and function-calling guide](https://github.com/QwenLM/Qwen-Agent)

This exercises a real agent loop with a replay transport and a synthetic business tool. It covers one framework version, one parser configuration and one checkpoint's previously saved generations. A deployed agent can choose a different parser, implement different argument validation or ask the model to repair rejected calls. This study supplies no second model response. Replaying both policies for all 40 responses took about 0.5 seconds on the test machine after imports. It made zero inference or paid API calls.

## Reproduce the offline replay

We used Python 3.13.7. From the repository root, install the optional study dependencies in a virtual environment, then run the check. The application and sidecar do not need these dependencies.

```sh
python3.13 -m venv /tmp/nexus-qwen-agent
/tmp/nexus-qwen-agent/bin/python -m pip install -r examples/exchange-approval/dispatch-study/framework/requirements.txt
/tmp/nexus-qwen-agent/bin/python examples/exchange-approval/dispatch-study/framework/replay.py --check
```

## Verify the spans in a disposable sidecar

The replay harness adds OpenTelemetry SDK spans at the public agent invocation, replay transport, registered callback and successful dispatch into SQLite. These are application-owned instrumentation points. Qwen-Agent supplies the parser and execution behavior. The replay marks the completion span as saved text and exports no prompt or response bodies. A trace records one pass with a specific enforcement policy. A thread groups both policies and all five input variants of the same synthetic request.

The check starts a private sidecar on an ephemeral loopback port with a temporary database. The SDK's batch processor sends OTLP/HTTP Protobuf. It then queries the sidecar's public API and checks every parent-child relationship. The measured run returned **8 threads, 80 traces and 236 spans**: 80 agent roots, 80 saved-completion spans, 60 callback spans and 16 SQLite dispatch spans. The receiver counted 80 normalized chat spans and zero captured input bodies, request snapshots or rule declarations. The 80 chat spans are replays of 40 saved strings across two policies.

```sh
make check-qwen-agent PYTHON=/tmp/nexus-qwen-agent/bin/python
```

To send the same traces to a sidecar you have already started on this machine, add `--sidecar-port 8040` to the offline replay command. Without that flag the SDK has no exporter. Span-name capture in the disposable test makes it possible to verify the tool and database spans from the trace detail endpoint.
