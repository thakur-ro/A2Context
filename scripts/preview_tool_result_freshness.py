"""Check archived tool values in the sidecar using a replayed origin digest."""

import argparse
from collections import Counter
import json
from pathlib import Path
from urllib.request import Request, urlopen

from replay_tool_result_retention import digest, load_call, require_loopback
from study_tool_result_retention import canonical

SOURCE = "tool-result"


def captured_value(sidecar, point, invocation):
    call = load_call(sidecar, point)
    result = next(item["outcome"] for item in call["tool_results"] if item["tool_call_id"] == invocation)
    if digest(result) != point["digest"]:
        raise ValueError(f"Captured payload digest changed at {point['span']}.")
    return result


def pending_request(tool, payload, expected_digest, anchor=True):
    text = canonical(payload)
    source = {"id": SOURCE, "source_id": SOURCE, "request_fragment": {
        "pointer": "/messages/0/content",
        "text_range": {"start_byte": 0, "end_byte": len(text.encode())},
    }}
    return {"request": {"version": 1, "attributes": {
        "nexusweave.request.body": canonical({"messages": [{"role": "tool", "content": text}]}),
        "nexusweave.context.semantics": {"version": 1, "request_sources": [source] if anchor else []},
        "nexusweave.required_sources": {
            "name": tool + " result",
            "any_of": [[SOURCE]],
            "expected_text_sha256": {SOURCE: expected_digest},
        },
    }}}


def check(sidecar, packet):
    request = Request(sidecar + "/api/sidecar/preview", canonical(packet).encode(),
                      {"Content-Type": "application/json"})
    with urlopen(request, timeout=60) as response:
        return json.load(response)["source_coverage"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", default="http://127.0.0.1:18058")
    parser.add_argument("--study", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    require_loopback(args.sidecar)
    sidecar = args.sidecar.rstrip("/")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = json.loads(args.study.read_text(encoding="utf-8"))
    counts = Counter()
    for case in report["examples"]:
        original = case["previous_result"]
        observed = case["later_result"]
        earlier = captured_value(sidecar, original, original["invocation"])
        later = captured_value(sidecar, observed, case["repeated_proposal"]["invocation"])
        same = case["result_comparison"] == "equal"
        for name, value, anchor, matched in (
            ("cached", earlier, True, same),
            ("refreshed", later, True, True),
            ("missing-anchor", later, False, False),
        ):
            # The later archived result serves as the simulated origin's reference.
            packet = pending_request(case["tool"], value, observed["digest"], anchor)
            coverage = check(sidecar, packet)
            if coverage["matched"] != matched or bool(coverage.get("text_mismatches")) != (anchor and not matched):
                raise ValueError(f"Unexpected source coverage for {case['workflow']}/{name}.")
            label = f"{case['workflow']}-{case['result_comparison']}-{name}.json"
            (args.output_dir / label).write_text(json.dumps(packet, indent=2) + "\n", encoding="utf-8")
            counts[name] += 1
    print("Verified preview packets", dict(counts), flush=True)


if __name__ == "__main__":
    main()
