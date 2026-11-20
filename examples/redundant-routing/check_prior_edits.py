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
    parser.add_argument("--candidate-checks", type=Path, help="Also verify and evaluate saved model scores for each input.")
    args = parser.parse_args()
    results = []
    source_arms = {"dispatch-guide": "guide", "routing-note": "note"}
    for case in json.loads(args.results.read_text())["cases"]:
        envelope = {"request": pending(case["parcel"], case["expected"], case["order"])}
        if args.candidate_checks:
            first = case["order"][0].lower().replace(" ", "-")
            saved = json.loads((args.candidate_checks / f'{case["parcel"].lower()}-{first}.json').read_text())
            assert saved["request"] == envelope["request"]
            envelope["candidate"] = saved["candidate"]
        req = Request(args.url.rstrip("/") + "/api/sidecar/preview", json.dumps(envelope).encode(),
                      {"Content-Type": "application/json"})
        with urlopen(req) as response:
            preview = json.load(response)
        history = preview["history"]
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
        if args.candidate_checks:
            scored = [r["detail"] for r in preview["recommendations"] if r["kind"] == "candidate"]
            assert len(scored) == 2 and not preview["unverified_probes"]
            assert all(d["replacement"] == condition["replacement"] for d in scored)
            selected = source_arms[scored[0]["sources"][0]["source"]]
            flips = [source_arms[d["sources"][0]["source"]] for d in scored if d["baseline"] * d["edited"] < 0]
            if flips:
                assert selected in flips, "The first recommendation must show a preference reversal."
            results[-1].update(model_first=selected, flagged=flips,
                               flagged_choice_changed=bool(flips) and condition[selected]["answer"] != baseline,
                               observed_changes=[k for k in ("guide","note") if condition[k]["answer"] != baseline])
    summary = {"requests": results, "count": len(results),
                      "changed_under_recommended_edit": sum(r["chosen_changed"] for r in results),
                      "changed_under_other_edit": sum(r["other_changed"] for r in results),
                      "baseline_correct": sum(r["baseline_correct"] for r in results)}
    if args.candidate_checks:
        summary.update(flagged_requests=sum(bool(r["flagged"]) for r in results),
                       changed_under_first_flagged_edit=sum(r["flagged_choice_changed"] for r in results),
                       changed_but_unflagged=sum(len(set(r["observed_changes"])-set(r["flagged"])) for r in results),
                       flagged_but_unchanged=sum(len(set(r["flagged"])-set(r["observed_changes"])) for r in results))
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
