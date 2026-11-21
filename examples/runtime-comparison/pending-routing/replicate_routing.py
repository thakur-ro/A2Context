"""Measure the fixed routing holdout locally, or export its saved observations as OTLP JSON."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from prefill_cohort import ATTRIBUTE, FOLDER, export, inputs

SCRIPTS = FOLDER.parents[2] / "scripts"
STUDY = FOLDER / "routing-holdout.json"
RESULTS = FOLDER / "routing-holdout-results.json"


def records(study):
    originals = json.loads((FOLDER / "shadow-cohort.json").read_text())
    original = next(r for r in originals if r["id"] == study["reference"])
    for case in study["inputs"]:
        meta = deepcopy(original["extra"]["metadata"])
        del meta["a2context.shadow_probes"]
        identifier = "routing-holdout-" + str(case["product"])
        meta["thread_id"] = identifier
        meta["a2context.span.id"] = identifier
        body = json.loads(meta["a2context.request.body"])
        body["messages"][2]["content"] = f"Product ID: {case['product']}."
        body["messages"][-1]["content"] = case["message"]
        meta["a2context.request.body"] = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
        yield dict(id=identifier, run_type="llm", name=original["name"], start_time=original["start_time"],
                   inputs={"messages": body["messages"]}, extra={"metadata": meta})


def measure(binary, model, study, cohort):
    source = json.loads((FOLDER / "prefill-cohort.json").read_text())[study["reference"]]
    scores = source["scores"]
    adapter = [model, "--model-id", scores["model_id"]]
    for record in cohort:
        with TemporaryDirectory() as tmp:
            folder = Path(tmp)
            request, edits, scored, result = [folder / name for name in ("request", "edits", "scored", "result")]
            request.write_text(json.dumps({"version": 1, "attributes": inputs(record)}))
            edits.write_text(json.dumps(source["interventions"]))
            subprocess.run([binary, "probe-prefill", request, edits, scored, "--", sys.executable,
                            SCRIPTS / "score_qwen_prefill.py", *adapter, "--revision", scores["model_revision"]], check=True)
            subprocess.run([binary, "test-routing", scored, FOLDER / "routing-design.json",
                            source["routing_test"]["runner_id"], result, "--", sys.executable,
                            SCRIPTS / "propose_qwen.py", *adapter], check=True)
            record["extra"]["metadata"][ATTRIBUTE] = json.loads(result.read_text())["attributes"][ATTRIBUTE]
    return cohort


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, help="Measure with this local checkpoint. Omit to export the saved corpus.")
    parser.add_argument("--binary", type=Path, default=Path("target/debug/a2context"))
    parser.add_argument("--output", type=Path, help="New file for the measured results.")
    args = parser.parse_args()
    study = json.loads(STUDY.read_text())
    cohort = list(records(study))
    if args.model:
        if not args.output or args.output.exists():
            parser.error("Choose a new output file for measurements.")
        measured = measure(args.binary.resolve(), args.model, study, cohort)
        with args.output.open("x") as output:
            json.dump(measured, output, indent=2)
            output.write("\n")
    else:
        if args.output:
            parser.error("Export writes OTLP JSON to stdout.")
        saved = json.loads(RESULTS.read_text())
        probes = {r["id"]: r["extra"]["metadata"][ATTRIBUTE] for r in saved}
        print(json.dumps(export(saved, probes)))
