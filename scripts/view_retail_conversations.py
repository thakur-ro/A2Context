"""Open the pinned public Exgentic retail sample in the sidecar conversation view."""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = "exgentic-tau2-retail"
FIELDS = ("gen_ai.operation.name", "gen_ai.request.model", "gen_ai.input.messages",
          "gen_ai.output.messages", "gen_ai.tool.definitions", "gen_ai.system_instructions")


def nanos(stamp):
    # The pinned archive has microsecond precision and no timezone suffix.
    delta = datetime.fromisoformat(stamp).replace(tzinfo=timezone.utc) - datetime(1970, 1, 1, tzinfo=timezone.utc)
    return str((delta.days * 86400 + delta.seconds) * 10**9 + delta.microseconds * 1000)


def attributes(values):
    return [{"key": key, "value": {"stringValue": value}} for key, value in values.items()]


def export_session(base, session, revision):
    session_id = session["session_id"]
    trace_id = uuid5(NAMESPACE_URL, f"Exgentic/agent-llm-traces/{revision}/{session_id}").hex
    spans = []
    for span in session["spans"]:
        values = {key: span["attributes"][key] for key in FIELDS if span["attributes"].get(key) is not None}
        values["gen_ai.conversation.id"] = session_id
        spans.append({"traceId": trace_id, "spanId": span["span_id"], "name": span["name"],
                      "startTimeUnixNano": nanos(span["start_time"]), "endTimeUnixNano": nanos(span["end_time"]),
                      "attributes": attributes(values)})
    batch = {"resourceSpans": [{"resource": {"attributes": attributes({"service.name": WORKFLOW})},
                                "scopeSpans": [{"spans": spans}]}]}
    body = json.dumps(batch, separators=(",", ":")).encode()
    with urlopen(Request(base + "/v1/traces", body, {"Content-Type": "application/json"}), timeout=60) as response:
        response.read()


def get(base, route, **params):
    with urlopen(base + "/api/sidecar/" + route + "?" + urlencode(params), timeout=30) as response:
        return json.load(response)


def check_content(dialogue, span):
    def parts(field, role):
        for message in json.loads(span["attributes"].get(field) or "[]"):
            if message["role"] == role:
                yield from message["parts"]

    def excerpt(text):
        return text if text is None or len(text) <= 360 else text[:240] + " … " + text[-120:]

    user_texts = [p["content"] for p in parts("gen_ai.input.messages", "user") if p["type"] == "text"]
    expected_user = excerpt(user_texts[-1]) if user_texts else None
    expected_outputs = []
    for part in parts("gen_ai.output.messages", "assistant"):
        if part["type"] == "text" and part["content"].strip():
            expected_outputs.append(("model_output", "Model answer", excerpt(part["content"])))
        elif part["type"] == "tool_call":
            args = part.get("arguments")
            if args is not None and not isinstance(args, str):
                args = json.dumps(args, separators=(",", ":"), ensure_ascii=False)
            expected_outputs.append(("tool_call", part["name"], excerpt(args)))
    shown = [(output["kind"], output["label"], output["text"]) for output in dialogue["outputs"]]
    if ((dialogue["user"] or {}).get("text") != expected_user or dialogue["output_count"] != len(expected_outputs)
            or shown != expected_outputs[:3]):
        raise ValueError("Stored dialogue differs from span " + span["span_id"] + ". Check --capture-content.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sample", type=Path)
    parser.add_argument("--url", default="http://127.0.0.1:8040", help="Sidecar receiving this public sample")
    parser.add_argument("--verify-content", action="store_true", help="Compare each displayed excerpt with the source messages")
    args = parser.parse_args()
    base = args.url.rstrip("/")
    manifest = json.loads((ROOT / "examples/retail-sample.manifest.json").read_text())
    content = args.sample.read_bytes()
    if sha256(content).hexdigest() != manifest["sha256"]:
        parser.error("sample digest differs from the pinned retail manifest")
    sessions = [json.loads(line) for line in content.splitlines()]
    for session in sessions:
        export_session(base, session, manifest["revision"])
    threads = {row["thread"]: row["id"] for row in get(base, "threads", workflow=WORKFLOW)["rows"]}
    for session in sessions:
        key = threads[session["session_id"]]
        spans = sorted(session["spans"], key=lambda span: span["start_time"])
        page = get(base, "thread-calls", thread=key)
        if [row["call"] for row in page["rows"]] != [s["span_id"] for s in spans] or page["next_offset"] is not None:
            raise ValueError("Stored call order differs from the pinned session " + session["session_id"])
        if args.verify_content:
            for row, span in zip(page["rows"], spans):
                check_content(row["dialogue"], span)
        print(f"{len(spans):2} calls  {base}/?" + urlencode({"view": "threads", "thread": key}))


if __name__ == "__main__":
    main()
