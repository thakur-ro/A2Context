"""Repeat one routing experiment across the fixed seeds in sampled-protocol.json."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from prefill_cohort import ATTRIBUTE, FOLDER, export, inputs
from replicate_routing import RESULTS, SCRIPTS

SAVED = FOLDER / "sampled-results.json"


def measure(binary, model):
    study = json.loads((FOLDER / "sampled-protocol.json").read_text())
    records = json.loads(RESULTS.read_text())
    record = next(r for r in records if r["id"] == study["reference"])
    metadata = record["extra"]["metadata"]
    original = metadata.pop(ATTRIBUTE)
    body = json.loads(metadata["a2context.request.body"])
    assert body["max_tokens"] == study["max_tokens"]
    body["temperature"] = study["temperature"]
    metadata.update(organization_id="routing-sampling", thread_id="sampled-routing-28")
    metadata["a2context.request.body"] = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
    adapter = [model, "--model-id", study["model"]]
    results = []
    with TemporaryDirectory() as directory:
        tmp = Path(directory)
        request, edits, scored = (tmp / name for name in ("request", "edits", "scored"))
        request.write_text(json.dumps({"version": 1, "attributes": inputs(record)}))
        edits.write_text(json.dumps(original["interventions"]))
        subprocess.run([binary, "probe-prefill", request, edits, scored, "--", sys.executable,
                        SCRIPTS / "score_qwen_prefill.py", *adapter, "--revision", study["revision"]], check=True)
        for seed in study["seeds"]:
            result = tmp / str(seed)
            subprocess.run([binary, "test-routing", scored, FOLDER / "routing-design.json", study["runner"],
                            result, "--", sys.executable, SCRIPTS / "propose_qwen.py", *adapter, "--seed", str(seed)], check=True)
            repeat = deepcopy(record)
            repeat["id"] = f"sampled-routing-28-seed-{seed}"
            meta = repeat["extra"]["metadata"]
            meta["a2context.span.id"] = repeat["id"]
            meta["sampling_seed"] = seed
            meta[ATTRIBUTE] = json.loads(result.read_text())["attributes"][ATTRIBUTE]
            results.append(repeat)
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/a2context"))
    parser.add_argument("--model", type=Path, help="Pinned local Qwen checkpoint for new measurements.")
    parser.add_argument("--output", type=Path, help="New file for all four sampled repeats.")
    args = parser.parse_args()
    if args.model:
        if not args.output or args.output.exists():
            parser.error("Choose a new output file for measurements.")
        measured = measure(args.binary.resolve(), args.model)
        with args.output.open("x") as output:
            json.dump(measured, output, ensure_ascii=False, indent=2)
            output.write("\n")
    else:
        if args.output:
            parser.error("Export writes OTLP JSON to stdout.")
        rows = json.loads(SAVED.read_text())
        print(json.dumps(export(rows, {row["id"]: row["extra"]["metadata"][ATTRIBUTE] for row in rows})))
