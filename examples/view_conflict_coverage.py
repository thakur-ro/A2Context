"""Reproduce three stages of source-edit coverage on saved Qwen journal requests."""
import argparse
from copy import deepcopy
import json
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from send_otlp_replay import prepare


def sessions():
    for topic, filename in [("Standard return", "report.json"), ("Policy question", "report-policy.json"),
                            ("Return follow-up", "report-followup.json")]:
        trace, report = prepare(topic, report_file=filename, archived=True, declare_windows=True)
        # Assign a shared demo tenant to the exported telemetry. The three model requests stay intact.
        span = trace["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        org = next(a for a in span["attributes"] if a["key"] == "nexusweave.organization.id")
        org["value"]["stringValue"] = "journal-coverage-lab"
        yield topic, trace, report


def submit(base, path, payload):
    with urlopen(Request(base + path, json.dumps(payload).encode(), {"Content-Type": "application/json"}), timeout=20):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    parser.add_argument("--finish", action="store_true", help="Also attach the withheld comparisons on the same calls")
    args = parser.parse_args()
    base = args.url.rstrip("/")
    for topic, trace, report in sessions():
        submit(base, "/v1/traces", trace)
        if topic == "Policy question":
            # Keep the untouched measured arm so the demo starts with only one side tested.
            single = deepcopy(report)
            single["id"] += "-policy-only"
            single.pop("pair", None)
            single["variants"] = [v for v in single["variants"] if v["intervention"]["context_id"] == "input:0:0"]
            submit(base, "/api/replay-reports", single)
        if topic == "Standard return" or args.finish:
            submit(base, "/api/replay-reports", report)
    location = report["location"]
    with urlopen(base + "/api/sidecar/findings?" + urlencode({"kind":"conflict","trace":location["run_id"]})) as response:
        card, = json.load(response)["rows"]
    params = {"recommendation":card["id"],"factor":card["evidence"]["detail"]["factor"],"conflict_coverage":"all"}
    if args.finish:
        with urlopen(base + "/api/sidecar/impact?org=journal-coverage-lab") as response:
            pattern = json.load(response)["conflict_pattern"]
        if pattern and pattern["group"] == card["id"] and pattern["factor"] == params["factor"]:
            params.pop("conflict_coverage")
            params.update(conflict_design=pattern["design"], org="journal-coverage-lab")
    print(base + "/?" + urlencode(params))


if __name__ == "__main__":
    main()
