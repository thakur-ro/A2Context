"""Exercise the dashboard with disposable sidecars and saved measurements."""
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from check_sidecar import ROOT, server


def main():
    binary = ROOT / "target/debug/a2context"
    for suite in ("sidecar_context_claims.py", "sidecar_shared_identifiers.py"):
        subprocess.run([sys.executable, ROOT / "tests" / suite, binary, "--browser"], check=True)
    with TemporaryDirectory() as tmp, server(binary, Path(tmp) / "onboarding.sqlite") as base:
        subprocess.run(["node", ROOT / "tests/onboarding-browser.mjs", base], check=True)
    with server(binary, None) as base:
        subprocess.run(["node", ROOT / "tests/exchange-a2ui-browser.mjs", base], check=True)


if __name__ == "__main__":
    main()
