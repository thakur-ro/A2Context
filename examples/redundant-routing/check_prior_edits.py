"""Check the demo's prospective source choices against saved held-out generations."""
import argparse
import json
from pathlib import Path
from urllib.request import Request, urlopen

from replicate_local import MODEL, RECORDS, request_for


def pending(parcel, bay, order):
    request = request_for(parcel, bay, order)
    sources = []
    for index, name in enumerate(order, 1):
        key = name.lower().replace(" ", "-")
        text = request["messages"][index]["content"]
        sources.append({"id": key, "source_id": key, "source_version": "1", "role": "user",
                        "request_fragment": {"pointer": f"/messages/{index}/content",
                                             "text_range": {"start_byte": len((name + ": ").encode()),
                                                            "end_byte": len(text.encode())}}})
    return {"version": 1, "attributes": {
        "gen_ai.request.model": MODEL, "nexusweave.organization.id": "dispatch-lab",
        "nexusweave.history.source": "synthetic-qwen-order",
        "nexusweave.request.body": json.dumps(request, separators=(",", ":")),
        "nexusweave.context.semantics": {"version": 1, "request_sources": sources}}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("heldout-order-results.json"))
    args = parser.parse_args()
    results = []
    source_arms = {"dispatch-guide": "guide", "routing-note": "note"}
    for case in json.loads(args.results.read_text())["cases"]:
        envelope = {"request": pending(case["parcel"], case["expected"], case["order"])}
        req = Request(args.url.rstrip("/") + "/api/sidecar/preview", json.dumps(envelope).encode(),
                      {"Content-Type": "application/json"})
        with urlopen(req) as response:
            history = json.load(response)["history"]
        top = history["edits"][0]
        assert top["requests"] == top["lower"] == 2 and not history["truncated"], top
        condition, = [c for c in case["conditions"] if c["replacement"] == top["example"]["replacement"]]
        chosen = source_arms[top["example"]["source"]]
        other = "note" if chosen == "guide" else "guide"
        baseline = case["baseline"]["answer"]
        results.append({"parcel": case["parcel"], "first": case["order"][0], "chosen": chosen,
                        "baseline_correct": baseline == case["expected"],
                        "chosen_changed": condition[chosen]["answer"] != baseline,
                        "other_changed": condition[other]["answer"] != baseline})
    print(json.dumps({"requests": results, "count": len(results),
                      "changed_under_recommended_edit": sum(r["chosen_changed"] for r in results),
                      "changed_under_other_edit": sum(r["other_changed"] for r in results),
                      "baseline_correct": sum(r["baseline_correct"] for r in results)}, indent=2))


if __name__ == "__main__":
    main()
