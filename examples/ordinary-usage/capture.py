"""Record requests to the pilot's local model. One listener per application run."""

from contextlib import contextmanager
from datetime import datetime, timezone
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
from threading import Thread, Lock
import time
from uuid import uuid4

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_sidecar import attrs, get, send_traces


def now():
    return datetime.now(timezone.utc).isoformat()


def otel_message(message):
    role = message["role"]
    parts = []
    content = message.get("content")
    if role == "tool":
        parts.append({"type": "tool_call_response", "id": message.get("tool_call_id"), "response": content})
    elif isinstance(content, str):
        parts.append({"type": "text", "content": content})
    elif isinstance(content, list):
        parts.extend(content)
    for call in message.get("tool_calls") or []:
        parts.append({"type": "tool_call", "id": call.get("id"), **call.get("function", {})})
    return {"role": role, "parts": parts}


@contextmanager
def capture(sidecar, model_port, workflow, run, journal):
    trace_id = uuid4().hex
    lock = Lock()
    seen = set()

    def append(event):
        with journal.open("a") as file:
            print(json.dumps({"at": now(), "run": run, "trace": "otlp:" + trace_id, **event}), file=file)

    class Capture(BaseHTTPRequestHandler):
        def do_POST(self):
            if self.path != "/v1/chat/completions":
                self.send_error(404)
                return
            raw = self.rfile.read(int(self.headers["Content-Length"]))
            request = json.loads(raw)
            started = time.time_ns()
            span_id = uuid4().hex[:16]
            conn = HTTPConnection("127.0.0.1", model_port, timeout=295)
            try:
                conn.request("POST", self.path, raw, {"Content-Type": "application/json"})
                response = conn.getresponse()
                status, content_type, body = response.status, response.getheader("Content-Type", ""), response.read()
            except OSError as exc:
                status, content_type, body = 502, "application/json", json.dumps({"error": str(exc)}).encode()
            finally:
                conn.close()
            ended = time.time_ns()
            fields = {"gen_ai.operation.name": "chat", "gen_ai.request.model": request["model"],
                      "gen_ai.conversation.id": run, "a2context.request.body": raw.decode(),
                      "gen_ai.input.messages": json.dumps([otel_message(m) for m in request["messages"]])}
            if request.get("tools"):
                fields["gen_ai.tool.definitions"] = json.dumps(request["tools"])
            reply = json.loads(body) if body and "application/json" in content_type else {}
            outputs = [otel_message(c["message"]) for c in reply.get("choices", []) if "message" in c]
            if outputs:
                fields["gen_ai.output.messages"] = json.dumps(outputs)
            span = {"traceId": trace_id, "spanId": span_id, "name": "chat", "startTimeUnixNano": str(started),
                    "endTimeUnixNano": str(ended), "attributes": attrs(fields)}
            if status >= 400:
                span["status"] = {"code": 2, "message": f"Provider HTTP {status}"}
                fields["error.type"] = str(status)
                span["attributes"] = attrs(fields)
            with lock:
                try:
                    send_traces(sidecar, {"resourceSpans": [{"resource": {"attributes": attrs({"service.name": workflow})},
                                                            "scopeSpans": [{"spans": [span]}]}]})
                    cards = get(sidecar, "findings")["rows"]
                    new = [c for c in cards if c["id"] not in seen]
                    seen.update(c["id"] for c in new)
                    append({"event": "capture", "span": span_id, "status": status,
                            "captured_output": bool(outputs), "cards": new})
                except Exception as exc:
                    append({"event": "capture_error", "span": span_id, "error": str(exc)})
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    # Complete any in-flight capture before moving to the next question.
    class Listener(ThreadingHTTPServer):
        daemon_threads = False

    with Listener(("127.0.0.1", 0), Capture) as listener:
        seen.update(c["id"] for c in get(sidecar, "findings")["rows"])
        append({"event": "started"})
        worker = Thread(target=listener.serve_forever, daemon=True)
        worker.start()
        try:
            yield f"http://127.0.0.1:{listener.server_port}/v1"
        finally:
            listener.shutdown()
            worker.join()
