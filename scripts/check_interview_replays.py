"""Check model-output and stored-state questions on two recorded experiments."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from time import time_ns

from check_sidecar import ROOT, attrs, get, send_traces, server

sys.path.insert(0, str(ROOT / "examples/exchange-approval"))
from demo import export


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("telecom_packet", type=Path, help="Saved telecom-refuel-measured.json")
    parser.add_argument("--endpoint", help="Optional Chat Completions endpoint")
    parser.add_argument("--model")
    args = parser.parse_args()
    if bool(args.endpoint) != bool(args.model):
        parser.error("Supply both --endpoint and --model for a partner run.")
    packet = json.loads(args.telecom_packet.read_text())
    fields = dict(packet["request"]["attributes"])
    fields.update({"gen_ai.operation.name": "chat", "gen_ai.conversation.id": "Refuel an existing line",
                   "nexusweave.shadow_probes": [packet["shadow"]]})
    now = time_ns()
    span = {"traceId": "16" * 16, "spanId": "34" * 8, "name": "Refuel check",
            "startTimeUnixNano": str(now), "endTimeUnixNano": str(now + 1000000),
            "attributes": attrs({key: value if isinstance(value, str) else json.dumps(value) for key, value in fields.items()})}
    batch = {"resourceSpans": [{"resource": {"attributes": attrs({"service.name": "telecom-refuel"})},
                                "scopeSpans": [{"spans": [span]}]}]}
    def check(origin):
        subprocess.run(["node", "scripts/check_interview_questions.mjs", origin, origin, args.model or "-", "replays"],
                       cwd=ROOT, check=True)
    with TemporaryDirectory() as folder, server(ROOT / "target/debug/nexusweave", Path(folder) / "replays.sqlite", "--capture-content") as base:
        export(base)
        send_traces(base, batch)
        for workflow in ["exchange-agent", "telecom-refuel"]:
            findings = get(base, "findings", workflow=workflow)["rows"]
            assert any(r["evidence"]["detail"].get("response_pair") and r["evidence"]["detail"].get("sandbox_dispatch")
                       for r in findings), f"Missing verified pair and receipt in {workflow}"
        if not args.model:
            return check(base)
        with subprocess.Popen([sys.executable, ROOT / "examples/chat-partner/serve.py", "--sidecar", base,
                               "--endpoint", args.endpoint, "--model", args.model, "--port", "0"],
                              stdout=subprocess.PIPE, text=True) as gateway:
            try:
                startup = gateway.stdout.readline().strip()
                assert startup.startswith("Open http://127.0.0.1:"), startup
                check(startup.split()[-1])
            finally:
                gateway.terminate()


if __name__ == "__main__":
    main()
