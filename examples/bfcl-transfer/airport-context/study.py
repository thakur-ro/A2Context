"""Test an optional pre-inference directory lookup against unchanged and withheld controls."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from inputs import digest, new_sim, select
from run import MODEL_FILES, RUNTIME, SELECTION_SHA, confirm, demo, load_runtime, save, summarize
from staging import SYSTEM, control, trial
from replay import instrumentation, logger


def directory(message, simulator):
    words = list(re.finditer(r"\b[A-Za-z]+\b", message))[:256]
    seen, matches = set(), {}
    supported = set(simulator.list_all_airports())
    for index, first in enumerate(words):
        for last in words[index:index + 3]:
            phrase = message[first.start():last.end()]
            if not re.fullmatch(r"[A-Za-z]+(?:\s+[A-Za-z]+)*", phrase):
                break
            if phrase in seen:
                continue
            seen.add(phrase)
            code = simulator.get_nearest_airport_by_city(phrase)["nearest_airport"]
            if code in supported:
                matches[phrase] = code
    return len(seen), matches


def instruction(matches, arm):
    return SYSTEM + "\n\nAirport directory:\n" + "\n".join(
        f"{city} = {code if arm == 'verified' else '[withheld]'}" for city, code in matches.items())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--upstream", type=Path, required=True)
    p.add_argument("--snapshot", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--repeat", type=Path)
    args = p.parse_args()
    if args.output.exists():
        p.error("Output path must be unused.")
    baseline_path = HERE.parent / "results.json"
    baseline = json.loads(baseline_path.read_text())
    selection, packets, schemas, cls = select(args.upstream)
    if selection != json.loads((HERE.parent / "selection.json").read_text()) or baseline["selection_sha256"] != SELECTION_SHA:
        p.error("Unexpected travel cohort.")
    demo.verify_checkpoint(args.snapshot)
    if any(sha256((args.snapshot / name).read_bytes()).hexdigest() != h for name, h in MODEL_FILES.items()):
        p.error("Unexpected checkpoint configuration.")
    rt = load_runtime(args.snapshot)
    if rt.identity != RUNTIME:
        p.error("Unexpected runtime.")
    saved = json.loads(args.repeat.read_text()) if args.repeat else None
    arms = ["withheld", "verified"]
    report = {"protocol_sha256":sha256((HERE / "protocol.md").read_bytes()).hexdigest(),
              "baseline_sha256":sha256(baseline_path.read_bytes()).hexdigest(), "cases":[]}
    costs = []
    with instrumentation() as tracer:
        for i, (pkt, base) in enumerate(zip(packets, baseline["cases"], strict=True)):
            if pkt["id"] != base["id"] or control(pkt, schemas, cls, tracer) != baseline["controls"][i]:
                raise ValueError("Baseline case or reference controls changed.")
            probes, matches = directory(pkt["messages"][0]["content"], new_sim(cls, pkt["state"]))
            row = {"id":pkt["id"], "candidate_phrases":probes, "matched_locations":len(matches),
                   "directory_sha256":digest(list(matches.items())), "arms":{}}
            for arm in arms:
                if not matches:
                    row["arms"][arm] = {"reused_baseline":True}
                    continue
                captures = []
                measured, _ = trial(pkt, schemas, cls, tracer, runtime=rt, tokenizer=rt.tokenizer,
                                    system_message=instruction(matches, arm), observations=captures)
                measured["domain_errors"] = sum(bool(c.get("returned_domain_error")) for c in captures)
                row["arms"][arm] = measured
                costs.append(measured)
            if saved:
                old = saved["cases"][i]
                if {k:v for k,v in old.items() if k != "arms"} != {k:v for k,v in row.items() if k != "arms"}:
                    raise ValueError("Directory or case identity changed.")
                for arm in arms:
                    if matches:
                        confirm(row["arms"][arm], old["arms"][arm], rt.tokenizer)
                    elif row["arms"][arm] != old["arms"][arm]:
                        raise ValueError("A reused baseline changed.")
            report["cases"].append(row)
            print(f"{pkt['id']}: {len(matches)} matched locations", flush=True)
    counts = {}
    for arm in arms:
        rows = [base if row["arms"][arm].get("reused_baseline") else row["arms"][arm]
                for row, base in zip(report["cases"], baseline["cases"], strict=True)]
        counts[arm] = {k:sum(int(r[k]) for r in rows) for k in ("usable","required_actions","required_responses","state_match")}
    report["outcomes"] = counts
    cost = summarize(costs)["cost"]
    if saved:
        if any(saved[k] != report[k] for k in ("protocol_sha256","baseline_sha256","outcomes")) or len(saved["cases"]) != len(report["cases"]):
            raise ValueError("Study identities or endpoints changed.")
        save(args.output, {"results_sha256":sha256(args.repeat.read_bytes()).hexdigest(),
                           "matched_cases":[r["id"] for r in report["cases"]],"cost":cost})
    else:
        save(args.output, {**report, "cost":cost})


if __name__ == "__main__":
    logger.setLevel("ERROR")
    main()
