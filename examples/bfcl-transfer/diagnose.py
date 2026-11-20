"""Recheck saved private BFCL completions and report field names and API dispositions without their values."""
import argparse
import json
from pathlib import Path

from inputs import new_sim, select
from run import confirm
from staging import trial
from replay import instrumentation, logger


def diagnose(root, public, raw, airport_fields=False):
    _, packets, schemas, cls = select(root)
    report = []
    with instrumentation() as tracer:
        for pkt, prior in zip(packets, public["cases"], strict=True):
            captures = []
            measured, _ = trial(pkt, schemas, cls, tracer, saved=raw[pkt["id"]], observations=captures)
            confirm(measured, prior, None)
            observed = []
            directory = new_sim(cls, pkt["state"]) if airport_fields else None
            for cap in captures:
                args = cap.pop("arguments", None)
                compared = []
                for i, (name, expected) in enumerate(pkt["ref"]):
                    if name == cap["name"] and args is not None:
                        differences = sorted(k for k in expected.keys() | args.keys() if
                                             k not in expected or k not in args or expected[k] != args[k])
                        comparison = {"reference_call_index": i, "different_argument_keys": differences}
                        if directory:
                            comparison["airport_fields"] = {
                                key: {"supported": args[key] in directory.list_all_airports(),
                                      "resolved_to_reference": directory.get_nearest_airport_by_city(args[key])["nearest_airport"] == expected[key]}
                                for key in ("travel_from", "travel_to") if key in differences and isinstance(args.get(key), str) and key in expected}
                        compared.append(comparison)
                observed.append({**cap, "matching_name_comparisons": compared})
            report.append({"id": pkt["id"], "calls": observed})
    return report


if __name__ == "__main__":
    logger.setLevel("ERROR")
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--upstream", type=Path, required=True)
    p.add_argument("--results", type=Path, required=True)
    p.add_argument("--audit", type=Path, required=True)
    p.add_argument("--airport-fields", action="store_true", help="Check differing airport values against the public directory.")
    args = p.parse_args()
    print(json.dumps(diagnose(args.upstream, json.loads(args.results.read_text()), json.loads(args.audit.read_text()), args.airport_fields), indent=2))
