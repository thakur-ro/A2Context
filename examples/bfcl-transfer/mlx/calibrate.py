"""Qualify each unchanged LFM travel workflow and verify an independent process repeat."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

from runner import HERE, controls, load_model, trial
from inputs import select
from run import confirm, save, summarize, SELECTION_SHA
from replay import logger


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("upstream","snapshot","output"):
        parser.add_argument("--"+flag, type=Path, required=True)
    parser.add_argument("--repeat", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Output path must be unused.")
    selection, packets, schemas, cls = select(args.upstream)
    meta = {"selection_sha256":SELECTION_SHA, "model":"LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit",
            "revision":"ce5150fd0de58d2bba8d88cc87f13e4fbda5977a",
            "protocol_sha256":sha256((HERE/"protocol.md").read_bytes()).hexdigest(),
            "files_sha256":sha256((HERE/"files.json").read_bytes()).hexdigest()}
    if sha256((HERE.parent/"selection.json").read_bytes()).hexdigest() != SELECTION_SHA or selection != json.loads((HERE.parent/"selection.json").read_text()):
        raise ValueError("Frozen case selection changed.")
    model, tokenizer = load_model(args.snapshot)
    checks = [controls(tokenizer, pkt, schemas, cls) for pkt in packets]
    old = json.loads(args.repeat.read_text()) if args.repeat else None
    if old and (old["controls"] != checks or old["design"] != meta or len(old["cases"]) != len(packets)):
        raise ValueError("Repeat design or simulator controls differ.")
    cases = []
    for i, pkt in enumerate(packets):
        row = trial(model, tokenizer, pkt, schemas, cls)
        if old:
            confirm(row, old["cases"][i], tokenizer)
        cases.append(row)
        print(pkt["id"], row["stop"], "qualified" if row["usable"] else "missed", flush=True)
    summary = summarize(cases)
    if old:
        save(args.output, {"results_sha256":sha256(args.repeat.read_bytes()).hexdigest(),
                           "matched":[p["id"] for p in packets],"cost":summary["cost"]})
    else:
        save(args.output, {"design":meta,"controls":checks,"cases":cases,"summary":summary})


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
