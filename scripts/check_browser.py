"""Exercise the dashboard with disposable sidecars and saved measurements."""
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from check_sidecar import ROOT, send, export as otlp_export, send_traces, server

sys.path.insert(0, str(ROOT / "examples/exchange-approval"))
from demo import export

sys.path.insert(0, str(ROOT / "tests"))
from sidecar_source_gaps import checked_span
from sidecar_baseline import baseline_report
from send_otlp_replay import prepare as prepare_replay


def main():
    binary = ROOT / "target/debug/a2context"
    subprocess.run([sys.executable, ROOT / "tests/sidecar_context_claims.py", binary, "--browser"], check=True)
    with TemporaryDirectory() as directory, server(binary, Path(directory) / "first-workflow.sqlite") as base:
        subprocess.run(["node", "tests/onboarding-browser.mjs", base], cwd=ROOT, check=True)
        subprocess.run(["node", "tests/tool-catalog-browser.mjs", base], cwd=ROOT, check=True)
    with server(binary, None) as base:
        subprocess.run(["node", "tests/interview-user-browser.mjs", base], cwd=ROOT, check=True)
        subprocess.run(["node", "tests/sidecar-browser.mjs"], cwd=ROOT, check=True,
                       env={**os.environ, "SIDECAR_URL": base})
        subprocess.run(["node", "tests/context-network-browser.mjs", base], cwd=ROOT, check=True)
    with subprocess.Popen([binary, "demo"], stdout=subprocess.PIPE, text=True) as demo:
        try:
            startup = demo.stdout.readline()
            assert startup.startswith("Open http://127.0.0.1:")
            base = startup.split()[-1]
            subprocess.run(["node", "tests/exchange-a2ui-browser.mjs", base], cwd=ROOT, check=True)
        finally:
            demo.terminate()
    with TemporaryDirectory() as directory, server(binary, Path(directory) / "gap.sqlite", "--capture-content") as base:
        export(base, capture_gap_binary=binary)
        send_traces(base, otlp_export(checked_span(binary, 1, user="digest-test", anchored=True, expected="c" * 64)))
        trace, report = prepare_replay("baseline-browser")
        send_traces(base, trace)
        send(base, "/api/replay-reports", baseline_report(report, "no_score"), expected=201)
        subprocess.run(["node", "tests/source-gap-browser.mjs", base], cwd=ROOT, check=True)
        subprocess.run(["node", "tests/tool-reference-browser.mjs", base], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
