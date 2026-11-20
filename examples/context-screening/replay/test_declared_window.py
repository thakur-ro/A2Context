"""Test a declared 45-day answer on three journal requests before dispatch."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def prepare(fixture, question=None):
    original = json.loads((HERE / fixture).read_text())
    body = original["request"]
    if question:
        body["messages"][-1]["content"] = question
    sources = []
    edits = []
    for index, (source, number) in enumerate([("journal-policy", "30"), ("journal-faq", "60")]):
        content = body["messages"][index]["content"]
        assert content.count(number + " days") == 1
        start = len(content[:content.index(number + " days")].encode())
        fragment = {"pointer": f"/messages/{index}/content",
                    "text_range": {"start_byte": start, "end_byte": start + len(number)}}
        sources.append({"id": source, "source_id": source, "source_version": "v1",
                        "request_fragment": fragment})
        edits.append({"context_id": "request-source:" + source, **fragment,
                      "operation": "replace_text_fragment", "replacement": "45"})
    request = {"version": 1, "attributes": {
        "gen_ai.request.model": body["model"],
        "nexusweave.request.body": json.dumps(body, separators=(",", ":"), ensure_ascii=False),
        "nexusweave.context.semantics": {"version": 1, "request_sources": sources}}}
    case = {"version": 1, "id": "declared-journal-window", "runner_id": original["runner_id"],
            "expected": {"kind": "model_output", "label": "Model answer", "outcome": "45"},
            "samples": 1, "interventions": edits, "test_pair": True}
    return request, case, body["model"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/nexusweave")
    args = parser.parse_args()
    args.output.mkdir(exist_ok=True, parents=True)
    cases = [("earlier", "case.json", None), ("followup", "case-followup.json", None),
             ("unopened", "case.json", "For an unopened journal, state the latest day on which I can request a return. Reply with the number only.")]
    for name, fixture, question in cases:
        request, case, model = prepare(fixture, question)
        with tempfile.TemporaryDirectory() as folder:
            paths = [Path(folder) / file for file in ("request.json", "case.json", "report.json")]
            for path, data in zip(paths[:2], (request, case), strict=True):
                path.write_text(json.dumps(data))
            subprocess.run([args.binary, "shadow", *paths, "--", sys.executable,
                            ROOT / "scripts/propose_qwen.py", args.model, "--model-id", model], check=True)
            report = json.loads(paths[2].read_text())
        (args.output / f"{name}-target.json").write_text(
            json.dumps({"request": request, "shadow": report}, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
