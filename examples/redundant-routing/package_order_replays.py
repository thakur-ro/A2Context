"""Bind saved local generations to exact requests using the Rust replay planner and scorer."""
import argparse
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

from replicate_local import MODEL, RECORDS, request_for


def source_id(record):
    return record.lower().replace(" ", "-")


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":")))


def package(binary, saved, tmp):
    runs, reports = [], []
    runner = " / ".join((saved["model"], saved["revision"], saved["runtime"], saved["tokenizer_sha256"]))
    if (saved["model"], saved["temperature"], saved["max_tokens"]) != (MODEL, 0, 16):
        raise ValueError("Saved model or decoding settings differ from the experiment.")
    for case in saved["cases"]:
        parcel, bay, order = case["parcel"], case["expected"], case["order"]
        request = request_for(parcel, bay, order)
        run_id = "order-" + parcel.lower() + "-" + source_id(order[0])
        location = {"run_id": run_id, "call_id": "dispatch", "decision_id": "bay"}
        choice = {"kind": "model_output", "label": "Model answer", "outcome": case["baseline"]["answer"]}
        sources = []
        for i, record in enumerate(order, 1):
            value = request["messages"][i]["content"]
            prefix = record + ": "
            assert value.startswith(prefix)
            sources.append({"id": source_id(record), "source_id": source_id(record), "source_version": "1",
                            "kind": "message", "role": "user", "availability": "included", "text": value[len(prefix):],
                            "request_fragment": {"pointer": f"/messages/{i}/content",
                                                 "text_range": {"start_byte": len(prefix.encode()), "end_byte": len(value.encode())}},
                            "evidence": {"field": "request.messages", "message_index": i}})
        call = {"id": "dispatch", "model": MODEL, "context": sources, "tool_results": [], "relationships": [],
                "recorded_request": {"body": json.dumps(request, separators=(",", ":")), "evidence": {"field": "request.body"}},
                "decisions": [{"id": "bay", **choice, "evidence": {"field": "generated_text"}}]}
        run = {"id": run_id, "source": "synthetic-qwen-order", "schema_version": 1, "organization_id": "dispatch-lab",
               "conversation_id": f"{parcel} with {order[0].lower()} first", "calls": [call]}
        runs.append(run)
        write(tmp / "evidence", [run])
        for index, condition in enumerate(case["conditions"]):
            wording = condition["replacement"]
            edits = [{"context_id": row["id"], **row["request_fragment"], "operation": "replace_text_fragment",
                      "replacement": wording} for row in sources]
            spec = {"version": 1, "id": f"{run_id}-{index + 1}", "runner_id": runner, "request": request,
                    "location": location, "samples": 1, "test_pair": True, "interventions": edits}
            write(tmp / "case", spec)
            plan_path, report_path = (tmp / (spec["id"] + suffix) for suffix in ["-plan", "-report"])
            subprocess.run([binary, "plan-replay", tmp / "evidence", tmp / "case", plan_path], check=True)
            plan = json.loads(plan_path.read_text())
            rows = []
            for arm in plan["conditions"]:
                edited = [record for record in RECORDS if source_id(record) in arm["context_ids"]]
                if json.loads(arm["body"]) != request_for(parcel, bay, order, edited, wording):
                    raise ValueError("Planned request differs from the local runner input.")
                results = {(): case["baseline"], (RECORDS[0],): condition["guide"],
                           (RECORDS[1],): condition["note"], RECORDS: condition["both"]}
                result = results[tuple(edited)]
                trial = {"status": "failed"}
                if "answer" in result:
                    trial = {"status": "completed", "choice": {**choice, "outcome": result["answer"]}}
                rows.append({"context_ids": arm["context_ids"], "request_sha256": arm["request_sha256"], "trials": [trial]})
            write(tmp / "trials", {"version": 1, "plan_id": plan["plan_id"], "conditions": rows})
            subprocess.run([binary, "score-replay", tmp / "evidence", tmp / "case", tmp / "trials", report_path], check=True)
            reports.append(json.loads(report_path.read_text()))
    return runs, reports


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("results", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    with TemporaryDirectory() as folder:
        runs, reports = package(args.binary.resolve(), json.loads(args.results.read_text()), Path(folder))
    args.output.write_text(json.dumps({"runs": runs, "reports": reports}, indent=2, ensure_ascii=False) + "\n")
