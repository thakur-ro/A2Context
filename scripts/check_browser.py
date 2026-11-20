"""Run both dashboard journeys with disposable sidecars and saved measurements."""
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from check_sidecar import ROOT, server, send_traces, export as otlp_export

sys.path.insert(0, str(ROOT / "examples/exchange-approval"))
from demo import export

sys.path.insert(0, str(ROOT / "tests"))
from sidecar_source_gaps import checked_span


def main():
    binary = ROOT / "target/debug/nexusweave"
    with server(binary, None) as base:
        subprocess.run(["node", "tests/sidecar-browser.mjs"], cwd=ROOT, check=True,
                       env={**os.environ, "SIDECAR_URL": base})
    with TemporaryDirectory() as directory, server(binary, Path(directory) / "exchange.sqlite", "--capture-content") as base:
        export(base)
        subprocess.run(["node", "tests/exchange-a2ui-browser.mjs", base], cwd=ROOT, check=True)
        send_traces(base, otlp_export(*(checked_span(binary, i) for i in (0, 1))))
        subprocess.run(["node", "tests/source-gap-browser.mjs", base], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
