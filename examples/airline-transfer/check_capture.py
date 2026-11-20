"""Check the frozen airline omissions with the same Rust preflight used by workflows."""
import argparse
import json
from pathlib import Path
import subprocess

from inputs import HERE, MARKER, canonical, cohort, digest


def source_ranges(case, arms):
    ranges = {}
    for name, anchor in case["anchors"].items():
        index = anchor["message_index_in_local_request"]
        original = arms["original"][index]["content"]
        before, after = arms[name][index]["content"].split(MARKER)
        assert original.startswith(before) and original.endswith(after)
        fragment = original[len(before):len(original) - len(after)]
        assert digest(fragment) == anchor["sha256"]
        ranges[name] = (index, len(before.encode()), len(fragment.encode()))
    return ranges


def pending_request(messages, tools, ranges, pins, edited=None, omit=False):
    sources = []
    for name, (index, start, length) in ranges.items():
        if name == edited:
            if omit:
                continue
            length = len(MARKER.encode())
        sources.append({"id": name, "source_id": name, "request_fragment": {
            "pointer": f"/messages/{index}/content",
            "text_range": {"start_byte": start, "end_byte": start + length}}})
    requirement = {"name": "Airline handoff", "any_of": [list(ranges)]}
    if pins:
        requirement["expected_text_sha256"] = pins
    attributes = {"nexusweave.request.body": canonical({"messages": messages, "tools": tools}),
                  "nexusweave.context.semantics": {"version": 1, "request_sources": sources},
                  "nexusweave.required_sources": requirement}
    return {"version": 1, "attributes": attributes}


def coverage(binary, messages, tools, ranges, pins, edited=None, omit=False):
    request = pending_request(messages, tools, ranges, pins, edited, omit)
    output = subprocess.check_output([binary, "preflight", "-"], text=True,
                                     input=json.dumps(request))
    report = json.loads(output)
    assert report["receipt"]["version"] == (3 if pins else 2)
    assert report["anchor_overlap"]["shared_bytes"] == 0
    return report["source_coverage"]


def measure(data, binary):
    manifest, packets, tools, _ = cohort(data)
    assert manifest == json.loads((HERE / "selection.json").read_text())
    rows = []
    for case, packet in zip(manifest["cases"], packets, strict=True):
        arms = packet["arms"]
        ranges = source_ranges(case, arms)
        pins = {name: anchor["sha256"] for name, anchor in case["anchors"].items()}
        for contract in ({}, pins):
            original = coverage(binary, arms["original"], tools, ranges, contract)
            assert original["matched"] and not original.get("text_mismatches")
        for name in ranges:
            by_id = coverage(binary, arms[name], tools, ranges, {}, name)
            by_text = coverage(binary, arms[name], tools, ranges, pins, name)
            absent = coverage(binary, arms[name], tools, ranges, pins, name, omit=True)
            assert by_id["matched"] and not by_text["matched"] and not absent["matched"]
            assert by_text["alternatives"][0]["unconfirmed"] == absent["alternatives"][0]["unconfirmed"] == [name]
            assert by_text["text_mismatches"] == [name] and not absent.get("text_mismatches")
            rows.append({"task": case["task"], "source": name, "id_only_passed": by_id["matched"],
                         "substitution_flagged": not by_text["matched"], "absent_anchor_flagged": not absent["matched"]})
    return {"selection_sha256": digest((HERE / "selection.json").read_text()),
            "intact_requests_passing_both_checks": len(packets), "edits": rows}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="Compare with capture-results.json")
    mode.add_argument("--request", nargs=2, metavar=("TASK", "SOURCE"),
                      help="Print a pinned substitution request for the sidecar preview")
    args = parser.parse_args()
    if args.request:
        task, source = args.request
        manifest, packets, tools, _ = cohort(args.data)
        assert manifest == json.loads((HERE / "selection.json").read_text())
        for case, packet in zip(manifest["cases"], packets, strict=True):
            if case["task"] == task and source in case["anchors"]:
                pins = {name: anchor["sha256"] for name, anchor in case["anchors"].items()}
                request = pending_request(packet["arms"][source], tools, source_ranges(case, packet["arms"]), pins, source)
                print(json.dumps({"request": request}, ensure_ascii=False))
                break
        else:
            parser.error("Choose a task and source from selection.json.")
        raise SystemExit(0)
    if not args.binary:
        parser.error("--binary is required for the capture comparison.")
    result = measure(args.data, args.binary.resolve())
    if args.check:
        assert result == json.loads((HERE / "capture-results.json").read_text())
        print(f"Verified {len(result['edits'])} substitutions and absent anchors against the frozen requests.")
    else:
        print(json.dumps(result, indent=2))
