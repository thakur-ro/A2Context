"""Recheck saved private BFCL completions and report field names and API dispositions without their values."""
import argparse
import json
from pathlib import Path

from inputs import select
from run import confirm
from staging import trial
from replay import instrumentation, logger


def diagnose(root, public, raw):
    _, packets, schemas, cls = select(root)
    report = []
    with instrumentation() as tracer:
        for pkt, prior in zip(packets, public["cases"], strict=True):
            captures = []
            measured, _ = trial(pkt, schemas, cls, tracer, saved=raw[pkt["id"]], observations=captures)
            confirm(measured, prior, None)
            observed = []
            for cap in captures:
                args = cap.pop("arguments", None)
                compared = []
                for i, (name, expected) in enumerate(pkt["ref"]):
                    if name == cap["name"] and args is not None:
                        differences = sorted(k for k in expected.keys() | args.keys() if
                                             k not in expected or k not in args or expected[k] != args[k])
                        compared.append({"reference_call_index": i, "different_argument_keys": differences})
                observed.append({**cap, "matching_name_comparisons": compared})
            report.append({"id": pkt["id"], "calls": observed})
    return report


if __name__ == "__main__":
    logger.setLevel("ERROR")
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--upstream", type=Path, required=True)
    p.add_argument("--results", type=Path, required=True)
    p.add_argument("--audit", type=Path, required=True)
    args = p.parse_args()
    print(json.dumps(diagnose(args.upstream, json.loads(args.results.read_text()), json.loads(args.audit.read_text())), indent=2))
