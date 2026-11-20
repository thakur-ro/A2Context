"""Build a Before inference upload by running the four source edits through the Rust shadow CLI."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

from cross import HERE, SCOPE, cohort, load_model, prepare, reconstruct, save, text_hash

MODEL = "LiquidAI/LFM2.5-1.2B-Instruct-MLX-8bit"


def build(args):
    parent = json.loads((HERE.parent / "results.json").read_text())
    case = next(c for c in parent["cases"] if c["id"] == "multi_turn_base_132")
    cases, full, cls = cohort(args.upstream)
    pkt = next(p for p in cases if p["id"] == case["id"])
    model, tokenizer = load_model(args.snapshot)
    arms = prepare(tokenizer, pkt, full, [t for t in full if t["name"] in SCOPE])
    histories = {k: reconstruct(model, tokenizer, pkt, arms[k][1], cls, case["arms"][k]) for k in ("full", "scoped")}
    messages = list(histories["full"])
    variant, tools, _ = arms[args.catalog]
    messages[0] = {"role": "system", "content": variant["system"]}
    body = {"model": MODEL, "messages": messages, "tools": tools, "max_tokens": 128, "temperature": 0}
    declarations, edits = [], []
    for i, name in [(2, "lookup-message"), (4, "update-message")]:
        span = {"pointer": f"/messages/{i}/content",
                "text_range": {"start_byte": 0, "end_byte": len(messages[i]["content"].encode())}}
        declarations.append({"id": name, "source_id": name, "source_version": "full-history", "request_fragment": span})
        edits.append({"context_id": "request-source:" + name, "operation": "replace_text_fragment",
                      **span, "replacement": histories["scoped"][i]["content"]})
    request = {"version": 1, "attributes": {"gen_ai.request.model": MODEL, "nexusweave.request.body": json.dumps(body),
               "nexusweave.context.semantics": {"version": 1, "request_sources": declarations}}}
    spec = {"version": 1, "id": "watchlist-132-" + args.catalog, "runner_id": "lfm2.5-1.2b-mlx-8bit-greedy",
            "expected": {"kind": "tool_call", "label": "get_watchlist", "outcome": {}}, "samples": 1,
            "interventions": edits, "test_pair": True}
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        save(root / "request.json", request)
        save(root / "case.json", spec)
        runner = HERE.parents[3] / "local-mlx-tool/runner.py"
        subprocess.run([args.binary, "shadow", root / "request.json", root / "case.json", root / "report.json", "--",
                        sys.executable, runner, args.snapshot], check=True)
        report = json.loads((root / "report.json").read_text())
    study = json.loads((HERE / "results.json").read_text())
    cells = {(c["first"], c["second"]): c for c in study["cells"] if c["catalog"] == args.catalog}
    groups = [report["baseline"], *(e["trials"] for e in report["effects"]), report["pair"]["trials"]]
    for key, group in zip([("full", "full"), ("scoped", "full"), ("full", "scoped"), ("scoped", "scoped")], groups, strict=True):
        trial, = group
        assert trial["status"] == "completed"
        choice, cell = trial["choice"], cells[key]
        assert ((cell["outcome"] == "get_watchlist" and choice == spec["expected"]) or
                (cell["outcome"] == "text" and choice["kind"] == "model_output" and text_hash(choice["outcome"]) == cell["raw_sha256"]))
    save(args.output, {"request": request, "allowed_tools": sorted(SCOPE), "shadow": report})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("upstream", "snapshot", "binary", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--catalog", choices=("full", "scoped", "dots", "letters"), default="scoped")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Choose an unused output file.")
    build(args)
