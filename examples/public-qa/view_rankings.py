"""Review the saved passage rankings and the independent replay of each selected edit."""
import argparse
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from urllib.request import urlopen

from check_passage_edits import DATA_SHA, dataset_records
from compare_orders import check_study
from view_passage_edits import post, prepare, verified_report, workflow_name


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nexusweave"))
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("holdout-results.json"))
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    args = parser.parse_args()
    study = json.loads(args.results.read_text())
    if study["dataset_sha256"] != DATA_SHA:
        raise ValueError("Study uses a different dataset.")
    check_study(study)
    base = args.url.rstrip("/")
    uploaded = 0
    completed = 0
    for (row, record), case in zip(dataset_records(args.dataset, study.get("cohort", "pilot")), study["cases"], strict=True):
        if row != case["row"]:
            raise ValueError("Dataset row differs from the measured input.")
        trace, replay, outcomes = prepare(record, case, study, rankings=True)
        location = replay["location"]
        report = None
        if case["baseline"] is not None:
            with TemporaryDirectory() as temp:
                report = verified_report(args.binary.resolve(), Path(temp), trace, replay, outcomes)
        post(base, "/v1/traces", trace)
        if report:
            post(base, "/api/replay-reports", report)
            completed += 1
        top = case["top_js"]
        context = f"request-source:passage-{top}"
        with urlopen(base + "/api/sidecar/trace?" + urlencode({"id": location["run_id"]}), timeout=30) as response:
            findings = json.load(response)["findings"]
        card, = [f for f in findings if f["kind"] == "prefill"]
        detail = card["detail"]
        if detail["ranked"][0]["context_id"] != context:
            raise ValueError("The sidecar selected a different passage.")
        seen = {c["context_id"]: c["changed"] for c in detail["proposal_checks"]
                if c["report"] == replay["id"] and c.get("decision") == location["decision_id"]}
        expected = {ids[0]: answer != case["baseline"] for ids, (_, answer) in outcomes.items()
                    if ids and answer is not None and case["baseline"] is not None}
        if seen != expected:
            raise ValueError("The comparisons verified by the sidecar differ from the saved generations.")
        print(f"Row {row}, first test: passage {top}, {case['edits'][top-1]['js_bits']:.4f} bits")
        selected = {"org": "public-qa", "recommendation": card["group"], "trace": location["run_id"],
                    "call": location["call_id"], "workflow":workflow_name(study), "ranking":"all"}
        print(f"{base}/?{urlencode(selected)}")
        if report:
            arm = urlencode({"v": 1, "view": "decisions", "run": location["run_id"], "call": location["call_id"],
                             "decision": location["decision_id"], "replay": report["id"], "variant": context})
            print(f"Exact trial: {base}/details?{urlencode({'run': location['run_id']})}#{arm}")
        else:
            print("The original answer reached the generation limit. Only the ranking is available.")
        uploaded += 1
    review = urlencode({"org":"public-qa", "workflow":workflow_name(study), "ranking":"all"})
    print(f"Loaded {uploaded} measured rankings and {completed} completed baseline answers. Open {base}/?{review}")


if __name__ == "__main__":
    main()
