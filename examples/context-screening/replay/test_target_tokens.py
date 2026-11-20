"""Measure next-token support for the declared journal answer and compare a fresh question."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from test_declared_window import HERE, ROOT, prepare

REVISION = "eaa56b503cc0a8a4d15de1dd8bd2a7e95a716be2"
FRESH = "How many days does the journal return policy allow? Give a number only."


def run_probe(binary, command, request, case, runner):
    with tempfile.TemporaryDirectory() as folder:
        paths = [Path(folder) / name for name in ("request.json", "case.json", "report.json")]
        for path, payload in zip(paths[:2], (request, case), strict=True):
            path.write_text(json.dumps(payload))
        subprocess.run([binary, command, *paths, "--", sys.executable, *runner], check=True, stdout=sys.stderr)
        return json.loads(paths[2].read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/nexusweave")
    args = parser.parse_args()
    args.output.mkdir(exist_ok=True, parents=True)
    scores = {}
    for name in ("earlier", "followup", "unopened", "fresh"):
        if name == "fresh":
            request, case, model = prepare("case.json", FRESH)
            shadow = run_probe(args.binary, "shadow", request, case,
                               [ROOT / "scripts/propose_qwen.py", args.model, "--model-id", model])
        else:
            saved = json.loads((HERE / f"{name}-target.json").read_text())
            request, shadow = saved["request"], saved["shadow"]
        case = {"version":1, "test_pair":True, "id":"journal-answer-prefix", "scorer_id":"qwen2-transformers-continuation-v1",
                "candidates":[{"id":"new-window","continuation":"45"}, {"id":"original-policy","continuation":"30"}],
                "interventions":[effect["intervention"] for effect in shadow["effects"]]}
        model = request["attributes"]["gen_ai.request.model"]
        score = run_probe(args.binary, "probe-candidates", request, case,
                          [ROOT / "scripts/score_qwen_candidates.py", args.model, "--model-id", model,
                           "--revision", REVISION, "--token-steps"])
        if name == "fresh":
            payload = {"request":request, "candidate":score, "shadow":shadow}
            (args.output / "fresh-target-tokens.json").write_text(json.dumps(payload, indent=2) + "\n")
        else:
            scores[name] = score
    (args.output / "target-token-scores.json").write_text(json.dumps(scores, indent=2) + "\n")


if __name__ == "__main__":
    main()
