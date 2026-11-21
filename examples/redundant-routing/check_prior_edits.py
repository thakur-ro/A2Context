"""Check the demo's prospective source choices against saved held-out generations."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.request import Request, urlopen

from replicate_local import MODEL, request_for


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
        "gen_ai.request.model": MODEL, "a2context.organization.id": "dispatch-lab",
        "a2context.history.source": "synthetic-qwen-order",
        "a2context.request.body": json.dumps(request, separators=(",", ":")),
        "a2context.context.semantics": {"version": 1, "request_sources": sources}}}


def reproduce_ash(model, binary, request, experiment):
    """Run the recommended edit through the CLI with the pinned local checkpoint."""
    folder = Path(__file__).parent
    scripts = folder.parents[1] / "scripts"
    saved = json.loads((folder / "ash-history-check.json").read_text())
    expected = saved["shadow"]
    case = {key: expected[key] for key in ("version", "id", "runner_id", "expected", "match_rule", "sampling_design")}
    case.update(samples=1, interventions=experiment["interventions"])
    with TemporaryDirectory() as scratch:
        path = Path(scratch)
        for name, value in (("request", request), ("edits", case["interventions"]), ("case", case)):
            (path / (name + ".json")).write_text(json.dumps(value))
        subprocess.run([
            binary, "probe-prefill", path / "request.json", path / "edits.json", path / "scored.json", "--",
            sys.executable, scripts / "score_qwen_prefill.py", model, "--model-id", MODEL,
            "--revision", saved["prefill"]["scores"]["model_revision"],
        ], check=True, stdout=sys.stderr)
        subprocess.run([
            binary, "shadow", path / "request.json", path / "case.json", path / "shadow.json", "--",
            sys.executable, scripts / "propose_qwen.py", model, "--model-id", MODEL,
        ], check=True, stdout=sys.stderr)
        actual = json.loads((path / "scored.json").read_text())["attributes"]["a2context.prefill.probe"]
        assert actual == saved["prefill"]
        assert json.loads((path / "shadow.json").read_text()) == expected
        assert actual["scores"]["baseline_request_sha256"] == experiment["baseline_request_sha256"]
        assert actual["scores"]["effects"][0]["request_sha256"] == experiment["edited_request_sha256"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("heldout-order-results.json"))
    parser.add_argument("--candidate-checks", type=Path, help="Also verify and evaluate saved model scores for each input.")
    parser.add_argument("--reproduce-ash", type=Path, metavar="MODEL", help="Rerun the saved Ash proposal pair and first-token measurement locally.")
    parser.add_argument("--binary", type=Path, default=Path("target/release/a2context"))
    args = parser.parse_args()
    results = []
    transfer = {"changed": {"changed": 0, "held": 0}, "held": {"changed": 0, "held": 0}, "no_consensus": 0}
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
        experiment = history["experiment"]
        prepared, = experiment["interventions"]
        assert prepared["context_id"] == "request-source:" + top["example"]["source"]
        assert prepared["replacement"] == top["example"]["replacement"]
        if args.reproduce_ash and case["parcel"] == "Ash" and case["order"][0] == "Routing note":
            reproduce_ash(args.reproduce_ash, args.binary.resolve(), envelope["request"], experiment)
        assert top["requests"] == top["lower"] == 2 and not history["truncated"], top
        condition, = [c for c in case["conditions"] if c["replacement"] == top["example"]["replacement"]]
        chosen = source_arms[top["example"]["source"]]
        other = "note" if chosen == "guide" else "guide"
        baseline = case["baseline"]["answer"]
        for edit in history["edits"]:
            observed, = [c for c in case["conditions"] if c["replacement"] == edit["example"]["replacement"]]
            source = source_arms[edit["example"]["source"]]
            actual = "changed" if observed[source]["answer"] != baseline else "held"
            prior = edit["proposals"]
            assert sum(prior.values()) == edit["requests"]
            if prior["changed"] == edit["requests"]:
                transfer["changed"][actual] += 1
            elif prior["held"] == edit["requests"]:
                transfer["held"][actual] += 1
            else:
                transfer["no_consensus"] += 1
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
    summary = {"requests": results, "count": len(results), "earlier_to_heldout_proposals": transfer,
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
