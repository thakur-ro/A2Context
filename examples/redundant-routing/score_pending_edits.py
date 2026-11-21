"""Score the declared bay code and UNKNOWN on each pending input and each source edit."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from check_prior_edits import pending
from replicate_local import MODEL, REPLACEMENTS

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def score_request(binary, snapshot, revision, request, key, alternatives, replacement):
    sources = request["attributes"]["a2context.context.semantics"]["request_sources"]
    spec = {"version": 1, "id": key, "scorer_id": "qwen2-transformers-continuation-v1",
            "assistant_prefix": "", "candidates": [{"id": v, "continuation": v} for v in alternatives],
            "interventions": [{"context_id": "request-source:" + s["id"], **s["request_fragment"],
                               "operation": "replace_text_fragment", "replacement": replacement} for s in sources]}
    with TemporaryDirectory() as directory:
        tmp = Path(directory)
        (tmp / "request").write_text(json.dumps(request))
        (tmp / "spec").write_text(json.dumps(spec))
        subprocess.run([binary, "probe-candidates", tmp / "request", tmp / "spec", tmp / "report", "--",
                        sys.executable, ROOT / "scripts/score_qwen_candidates.py", snapshot, "--model-id", MODEL,
                        "--revision", revision], check=True)
        return json.loads((tmp / "report").read_text())


def measure(binary, snapshot, revision, output):
    cases = json.loads((HERE / "heldout-order-results.json").read_text())["cases"]
    output.mkdir(exist_ok=False, parents=True)
    for case in cases:
        parcel, bay, order = case["parcel"], case["expected"], case["order"]
        request = pending(parcel, bay, order)
        key = parcel.lower() + "-" + order[0].lower().replace(" ", "-")
        report = score_request(binary, snapshot, revision, request, key, (bay, "UNKNOWN"), REPLACEMENTS[0])
        (output / (key + ".json")).write_text(json.dumps({"request": request, "candidate": report}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    measure(args.binary.resolve(), args.snapshot, args.revision, args.output)
