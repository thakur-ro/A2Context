"""Generate, independently repeat or check BFCL multi-turn first-turn baseline receipts."""
import argparse
from collections import Counter
from importlib.metadata import version
from hashlib import sha256
import json
from pathlib import Path

from inputs import HERE, select
from staging import SOURCES, control, trial
from replay import demo, instrumentation, logger
from study import REVISION, RUNTIME, load_runtime
from score_qwen_prefill import js_bits

SELECTION_SHA = "4bd0ec0df4df1b24ba1c1bd18ed668fb295a67cd285bf04399b54fdfc4f10109"
MODEL_FILES = {
    "config.json": "18e18afcaccafade98daf13a54092927904649e1dd4eba8299ab717d5d94ff45",
    "generation_config.json": "e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6",
    "tokenizer_config.json": "006da433cdc0e70f05759008ef5d8d82721e3cbd95bd4831d394f96c54230680",
}


def save(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write("\n")


def summarize(rows):
    counts, cost = Counter(), Counter()
    for row in rows:
        counts["cases"] += 1
        counts[row["stop"]] += 1
        for key in ("state_match", "required_actions", "required_responses", "usable"):
            counts[key] += int(row[key])
        for event in row["calls"]:
            counts[event["status"]] += 1
        for g in row["generations"]:
            cost.update(calls=1, input_tokens=g["input_tokens"], generated_tokens=g["generated_tokens"],
                        decode_seconds=g["decode_seconds"])
    return {"outcomes": counts, "cost": cost}


def confirm(measured, saved, tokenizer):
    actual = {k: v for k, v in measured.items() if k != "generations"}
    expected = {k: v for k, v in saved.items() if k != "generations"}
    if actual != expected or len(measured["generations"]) != len(saved["generations"]):
        raise ValueError(f"Native action or simulator state differs for {saved['id']}.")
    for new, old in zip(measured["generations"], saved["generations"], strict=True):
        keys = {"history_sha256", "raw_sha256", "stop", "generated_tokens"}
        if tokenizer:
            keys.update(["prompt_sha256", "input_tokens"])
        if any(new[k] != old[k] for k in keys):
            raise ValueError(f"Model receipts differ for {saved['id']}.")


def ranking(packets, schemas, cls, rows, rt, tracer):
    report = []
    for pkt, original in zip(packets, rows, strict=True):
        if not original["usable"]:
            continue
        baseline, base_receipt = trial(pkt, schemas, cls, tracer, runtime=rt, tokenizer=rt.tokenizer, probe=True)
        if base_receipt["prompt_sha256"] != original["generations"][0]["prompt_sha256"]:
            raise ValueError("Probe source differs from independently repeated baseline.")
        scores = {}
        for source in SOURCES:
            branch, receipt = trial(pkt, schemas, cls, tracer, runtime=rt, tokenizer=rt.tokenizer, probe=True, source=source)
            scores[source] = {**receipt, "js_bits": js_bits(rt.torch, baseline, branch)}
        top = max(SOURCES, key=lambda name: scores[name]["js_bits"])
        variants = {}
        for choice in dict.fromkeys((top, "customer")):
            variants[choice], _ = trial(pkt, schemas, cls, tracer, runtime=rt, tokenizer=rt.tokenizer, source=choice)
        report.append({"id": pkt["id"], "original_probe": base_receipt, "scores": scores, "ranked": top,
                       "latest": "customer", "same_selector": top == "customer", "variants": variants})
    return {"baseline_cases": len(rows), "usable_cases": len(report), "probe_calls": len(report) * 4,
            "edited_workflows": sum(len(x["variants"]) for x in report), "cases": report}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--upstream", type=Path, required=True)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--check", type=Path)
    mode.add_argument("--repeat", type=Path)
    mode.add_argument("--rank", type=Path)
    p.add_argument("--verification", type=Path)
    p.add_argument("--audit", type=Path, help="Private local raw responses for rechecking the actual parser.")
    p.add_argument("--snapshot", type=Path)
    p.add_argument("--output", type=Path)
    args = p.parse_args()
    if version("qwen-agent") != "0.0.34":
        p.error("Use Qwen-Agent 0.0.34.")
    manifest, packets, schemas, cls = select(args.upstream)
    frozen = (HERE / "selection.json").read_bytes()
    if sha256(frozen).hexdigest() != SELECTION_SHA or json.loads(frozen) != manifest:
        raise ValueError("The upstream reference does not reproduce the frozen cohort.")
    source = args.check or args.repeat or args.rank
    saved = json.loads(source.read_text()) if source else None
    if saved and (saved["selection_sha256"] != SELECTION_SHA or saved["summary"] != summarize(saved["cases"])
                  or [r["id"] for r in saved["cases"]] != [p["id"] for p in packets]):
        raise ValueError("Invalid cohort or saved totals.")
    if not args.check and (not args.output or args.output.exists() or (args.audit and args.audit.exists())):
        p.error("New experiments require unused output paths.")
    with instrumentation() as tracer:
        controls = [control(pkt, schemas, cls, tracer) for pkt in packets]
        if args.check:
            if args.output or saved["controls"] != controls:
                p.error("Unexpected output path or simulator controls.")
            if args.audit:
                private = json.loads(args.audit.read_text())
                tokenizer = None
                if args.snapshot:
                    from transformers import AutoTokenizer
                    tokenizer = AutoTokenizer.from_pretrained(args.snapshot, local_files_only=True, trust_remote_code=False)
                for pkt, prior in zip(packets, saved["cases"], strict=True):
                    current, _ = trial(pkt, schemas, cls, tracer, saved=private[pkt["id"]], tokenizer=tokenizer)
                    confirm(current, prior, tokenizer)
            print("Verified frozen reference controls and summary" + (" and private parser/state replay." if args.audit else "."))
            return
        source_sha = sha256(source.read_bytes()).hexdigest() if source else None
        if args.rank:
            if not args.verification:
                p.error("Ranking requires an independent --verification receipt.")
            verified = json.loads(args.verification.read_text())
            if verified["results_sha256"] != source_sha or verified["matched"] != [pkt["id"] for pkt in packets]:
                p.error("Repeat does not match the entire saved cohort.")
            if not saved["summary"]["outcomes"]["usable"]:
                save(args.output, {"results_sha256": source_sha, **ranking(packets, schemas, cls, saved["cases"], None, tracer)})
                return
        if not args.snapshot:
            p.error("Fresh inference requires the pinned local snapshot.")
        demo.verify_checkpoint(args.snapshot)
        if any(sha256((args.snapshot / fn).read_bytes()).hexdigest() != sha for fn, sha in MODEL_FILES.items()):
            p.error("Unexpected checkpoint configuration.")
        rt = load_runtime(args.snapshot)
        if rt.identity != RUNTIME:
            p.error("Unexpected local runtime.")
        if args.rank:
            save(args.output, {"results_sha256": source_sha, **ranking(packets, schemas, cls, saved["cases"], rt, tracer)})
            return
        rows, audit = [], {}
        for i, pkt in enumerate(packets):
            row, audit[pkt["id"]] = trial(pkt, schemas, cls, tracer, runtime=rt, tokenizer=rt.tokenizer)
            if saved:
                confirm(row, saved["cases"][i], rt.tokenizer)
            rows.append(row)
            print(f"Finished {pkt['id']}: {row['stop']} after {len(row['generations'])} generation(s).", flush=True)
        if args.repeat:
            save(args.output, {"results_sha256": source_sha, "matched": [r["id"] for r in rows],
                               "cost": summarize(rows)["cost"]})
        else:
            save(args.output, {"selection_sha256": SELECTION_SHA, "model": demo.MODEL, "revision": REVISION,
                               "runtime": RUNTIME, "parser": "Qwen-Agent 0.0.34, Nous", "cases": rows,
                               "controls": controls, "summary": summarize(rows)})
        if args.audit:
            save(args.audit, audit)


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
