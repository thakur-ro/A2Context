"""Load the public QA study into a running sidecar using saved generations."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

from check_passage_edits import DATA_SHA, dataset_records, ordered_documents, request, summarize


def prepare(record, case, study):
    documents = ordered_documents(case["row"], record, study["rotation"])
    question = record["question"]
    passages = [doc["title"] + "\n" + doc["text"] for doc in documents]
    if sha256(question.encode()).hexdigest() != case["question_sha256"]:
        raise ValueError("Question differs from the measured input.")

    def body(removed):
        return {"model": study["model"], "messages": request(question, passages, removed),
                "temperature": 0, "max_tokens": 48}

    original = body(None)
    raw = original["messages"][1]["content"].encode()
    sources, edits, outcomes = [], [], {}
    cursor = 0
    for i, (doc, passage, measured) in enumerate(zip(documents, passages, case["edits"], strict=True)):
        digest = sha256(passage.encode()).hexdigest()
        if measured["position"] != i + 1 or measured["passage_sha256"] != digest:
            raise ValueError("Passage content or ordering differs from the measured input.")
        prefix = (("\n\n" if i else "") + f"[{i + 1}] ").encode()
        start = cursor + len(prefix)
        cursor = start + len(passage.encode())
        if raw[start - len(prefix):cursor] != prefix + passage.encode():
            raise ValueError("Selected byte range differs from the measured passage.")
        fragment = {"pointer": "/messages/1/content", "text_range": {"start_byte": start, "end_byte": cursor}}
        source = f"passage-{i + 1}"
        sources.append({"id": source, "source_id": doc["title"], "source_version": digest,
                        "role": "user", "request_fragment": fragment})
        context_id = "request-source:" + source
        edits.append({"context_id": context_id, **fragment, "operation": "replace_text_fragment",
                      "replacement": "[passage omitted]"})
        outcomes[(context_id,)] = (body(i), measured["answer"])
    outcomes[()] = (original, case["baseline"])

    identity = [DATA_SHA, str(case["row"]), str(study["rotation"]), str(study["finished_unix_ns"]),
                study["model"], study["revision"], study["runtime"], study["tokenizer_sha256"]]
    trace_id = uuid5(NAMESPACE_URL, "/".join(identity)).hex
    span_id = trace_id[:16]
    attributes = {
        "gen_ai.operation.name": "chat", "gen_ai.request.model": study["model"],
        "gen_ai.conversation.id": f"QA {case['row']} · {question}",
        "nexusweave.organization.id": "public-qa",
        "nexusweave.request.body": json.dumps(original, ensure_ascii=False, separators=(",", ":")),
        "nexusweave.context.semantics": json.dumps({"version": 1, "request_sources": sources}),
        "gen_ai.input.messages": json.dumps([{"role": message["role"], "parts": [
            {"type": "text", "content": message["content"]}]} for message in original["messages"]]),
        "gen_ai.output.messages": json.dumps([{"role": "assistant", "parts": [
            {"type": "text", "content": case["baseline"]}]}]),
    }
    timestamp = study["finished_unix_ns"]
    span = {"traceId": trace_id, "spanId": span_id, "name": "Public QA study replay",
            "startTimeUnixNano": str(timestamp), "endTimeUnixNano": str(timestamp),
            "attributes": [{"key": key, "value": {"stringValue": value}} for key, value in attributes.items()]}
    trace = {"resourceSpans": [{"scopeSpans": [{"spans": [span]}]}]}
    replay = {"version": 1, "id": "public-qa-" + trace_id, "samples": 1,
              "runner_id": " / ".join(study[key] for key in ("model", "revision", "runtime", "tokenizer_sha256")),
              "location": {"run_id": "otlp:" + trace_id, "call_id": span_id, "decision_id": "output:0:0"},
              "request": original, "interventions": edits}
    return trace, replay, outcomes


def verified_report(binary, directory, trace, replay, outcomes):
    def run(*args):
        subprocess.run([binary, *args], cwd=directory, check=True, stdout=subprocess.DEVNULL)

    def save(name, data):
        (directory / name).write_text(json.dumps(data, ensure_ascii=False))

    save("trace", trace)
    save("case", replay)
    run("import-otlp", "trace", "evidence")
    run("plan-replay", "evidence", "case", "plan")
    plan = json.loads((directory / "plan").read_text())
    rows = []
    for arm in plan["conditions"]:
        expected, answer = outcomes[tuple(arm["context_ids"])]
        if json.loads(arm["body"]) != expected:
            raise ValueError("The Rust planner produced a different request than the study.")
        trial = {"status": "failed"} if answer is None else {
            "status": "completed", "choice": {"kind": "model_output", "label": "Model answer", "outcome": answer}}
        rows.append({"context_ids": arm["context_ids"], "request_sha256": arm["request_sha256"], "trials": [trial]})
    save("trials", {"version": 1, "plan_id": plan["plan_id"], "conditions": rows})
    run("score-replay", "evidence", "case", "trials", "report")
    return json.loads((directory / "report").read_text())


def post(base, path, data):
    message = Request(base + path, json.dumps(data).encode(), {"Content-Type": "application/json"})
    with urlopen(message, timeout=30) as response:
        response.read()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nexusweave"))
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("results.json"))
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    args = parser.parse_args()
    study = json.loads(args.results.read_text())
    if study["dataset_sha256"] != DATA_SHA or summarize(study["cases"]) != study["summary"]:
        raise ValueError("Study identity or summary differs from saved measurements.")
    base = args.url.rstrip("/")
    uploaded = 0
    for (row, record), case in zip(dataset_records(args.dataset, study.get("cohort", "pilot")), study["cases"], strict=True):
        if row != case["row"]:
            raise ValueError("Dataset row differs from the measured input.")
        if case["baseline"] is None:
            continue
        trace, replay, outcomes = prepare(record, case, study)
        with TemporaryDirectory() as temp:
            report = verified_report(args.binary.resolve(), Path(temp), trace, replay, outcomes)
        post(base, "/v1/traces", trace)
        post(base, "/api/replay-reports", report)
        location = report["location"]
        top = case["top_js"]
        context = f"request-source:passage-{top}"
        with urlopen(base + "/api/sidecar/trace?" + urlencode({"id": location["run_id"]}), timeout=30) as response:
            findings = json.load(response)["findings"]
        card = next((f for f in findings if f["kind"] == "replay"
                     and f["detail"]["measurement"]["replay_id"] == report["id"]
                     and f["detail"]["sources"][0]["id"] == context), None)
        print(f"Row {row}, first test: passage {top}, {case['edits'][top-1]['js_bits']:.4f} bits")
        if card:
            selected = {"org": "public-qa", "recommendation": card["group"], "trace": location["run_id"],
                        "call": location["call_id"], "replay": report["id"], "context": context}
            print(f"{base}/?{urlencode(selected)}")
            arm = urlencode({"v": 1, "view": "decisions", "run": location["run_id"], "call": location["call_id"],
                             "decision": location["decision_id"], "replay": report["id"], "variant": context})
            print(f"Exact trial: {base}/details?{urlencode({'run': location['run_id']})}#{arm}")
        else:
            print("This replay has no measured comparison for the suggested passage.")
        uploaded += 1
    print(f"Loaded {uploaded} recorded answers. Open {args.url.rstrip('/')}/?org=public-qa")


if __name__ == "__main__":
    main()
