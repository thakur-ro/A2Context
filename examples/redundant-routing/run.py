"""Repeat the synthetic two-source experiment. Uses nine short OpenAI requests."""
import argparse
from datetime import datetime, timezone
import http.client
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

MODEL = "gpt-4o-mini-2024-07-18"
SOURCES = ["dispatch-guide", "routing-note"]
RULE = "Route Alder parcels to bay LARK."
REQUEST = {
    "model": MODEL, "temperature": 0, "max_tokens": 16, "store": False,
    "messages": [
        {"role": "system", "content": "Use the supplied dispatch records. Reply with the bay code alone. If no record specifies a bay, reply UNKNOWN."},
        {"role": "user", "content": RULE},
        {"role": "user", "content": RULE},
        {"role": "user", "content": "Where should an Alder parcel go?"},
    ],
}


def trial(connection, key, body, usage):
    connection.request("POST", "/v1/chat/completions", body.encode(),
                       {"Content-Type": "application/json", "Authorization": "Bearer " + key})
    response = connection.getresponse()
    data = json.loads(response.read())
    if response.status != 200:
        raise RuntimeError(f"OpenAI returned HTTP {response.status}")
    choice = data["choices"][0]
    if choice["finish_reason"] != "stop" or not isinstance(choice["message"]["content"], str):
        raise RuntimeError("Incomplete answer")
    usage.append({"input": data["usage"]["prompt_tokens"], "output": data["usage"]["completion_tokens"]})
    return {"kind": "model_output", "label": "Model answer", "outcome": choice["message"]["content"]}


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def run_comparison(binary, output, evidence, call_id, run_id, conn, key, usage, name, replacement):
    case = output / (name + "-case.json")
    save(case, {"version": 1, "id": "two-routing-sources-" + name, "runner_id": MODEL + " / temperature 0",
                "location": {"run_id": run_id, "call_id": call_id, "decision_id": "bay"},
                "request": REQUEST, "samples": 1, "test_pair": True,
                "interventions": [{"context_id": source, "pointer": f"/messages/{i}/content",
                                   "replacement": replacement} for i, source in enumerate(SOURCES, 1)]})
    with TemporaryDirectory() as folder:
        plan_path, trials_path = (Path(folder) / name for name in ["plan.json", "trials.json"])
        subprocess.run([binary, "plan-replay", evidence, case, plan_path], check=True)
        plan = json.loads(plan_path.read_text())
        assert len(plan["conditions"]) == 4 and plan["samples"] == 1
        rows = []
        for condition in plan["conditions"]:
            choice = trial(conn, key, condition["body"], usage)
            rows.append({"context_ids": condition["context_ids"], "request_sha256": condition["request_sha256"],
                         "trials": [{"status": "completed", "choice": choice}]})
        save(trials_path, {"version": 1, "plan_id": plan["plan_id"], "conditions": rows})
        subprocess.run([binary, "score-replay", evidence, case, trials_path, output / (name + "-report.json")], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory for the results")
    args = parser.parse_args()
    key = os.environ["OPENAI_API_KEY"]
    args.output.mkdir(parents=True, exist_ok=False)
    conn = http.client.HTTPSConnection("api.openai.com", timeout=30)
    usage = []
    body = json.dumps(REQUEST, separators=(",", ":"))
    started = datetime.now(timezone.utc).isoformat()
    decision = trial(conn, key, body, usage)
    call_id, run_id = "dispatch", "redundant-routing"
    sources = [{"id": name, "source_id": name, "source_version": "1", "kind": "message",
                "role": "user", "availability": "included", "text": RULE,
                "evidence": {"field": "request.messages", "message_index": index}}
               for index, name in enumerate(SOURCES, 1)]
    call = {"id": call_id, "model": MODEL, "started_at": started, "context": sources,
            "recorded_request": {"body": body, "evidence": {"field": "request.body"}},
            "decisions": [{"id": "bay", **decision, "evidence": {"field": "choices.0.message.content"}}],
            "tool_results": [], "relationships": []}
    evidence = args.output / "evidence.json"
    save(evidence, [{"id": run_id, "source": "synthetic-openai", "schema_version": 1,
                     "conversation_id": "Alder parcel", "calls": [call]}])
    for name, replacement in [("unavailable", "No dispatch record available."),
                              ("neutral", "This parcel was scanned this morning.")]:
        run_comparison(args.binary, args.output, evidence, call_id, run_id, conn, key, usage, name, replacement)
    conn.close()
    print(f"{len(usage)} calls, {sum(row['input'] for row in usage)} input tokens, {sum(row['output'] for row in usage)} output tokens")


if __name__ == "__main__":
    main()
