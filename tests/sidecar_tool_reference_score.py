"""Check the recorded tool-reference score against distinct traces sent through OTLP."""
import json
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_sidecar import attrs, case_span, export, get, send_traces, server


def call(n, messages=None, user="alice", normalized=None):
    span = case_span("tool-reference-score", "undeclared", n, shift=0)
    fields = {"gen_ai.operation.name": "chat", "gen_ai.request.model": "fixture",
              "a2context.user.id": user, "gen_ai.conversation.id": f"thread-{n}"}
    if messages is not None:
        fields["a2context.request.body"] = json.dumps({"messages": messages})
    if normalized:
        fields["gen_ai.input.messages"] = json.dumps(normalized)
    span["attributes"] = attrs(fields)
    return span


def verify(binary):
    invoke = {"role": "assistant", "tool_calls": [{"id": "read", "type": "function",
              "function": {"name": "lookup", "arguments": "{}"}}]}
    result = {"role": "tool", "tool_call_id": "read", "content": "value"}
    user_msg = {"role": "user", "content": "look it up"}
    valid = [user_msg, invoke, result]
    orphan = [user_msg, result]
    affected = call(2, orphan)
    unresolved = [{"role": "tool", "parts": [{"type": "tool_call_response", "id": "read", "response": "value"}]}]
    spans = [call(1, valid), affected, call(3, normalized=unresolved), call(4), call(5, [user_msg]),
             call(6, [invoke, invoke, result], user="bob")]
    late = call(1)
    late["traceId"] = spans[0]["traceId"]
    late["spanId"] = "7777777777777777"
    second_failure = call(2, orphan)
    second_failure["traceId"] = affected["traceId"]
    second_failure["spanId"] = "8888888888888888"
    with TemporaryDirectory() as tmp:
        db = Path(tmp) / "score.sqlite"
        with server(binary, db, "--capture-content", "--retain-days", "365") as base:
            assert get(base, "tool-reference-score")["pass_pct"] is None
            send_traces(base, export(*spans))
            send_traces(base, export(*spans))
            score = get(base, "tool-reference-score", user="alice")
            assert [score[k] for k in ("clear", "affected", "unresolved", "outside", "unknown_applicability")] == [1, 1, 1, 1, 1], score
            assert score["pass_pct"] == 50 and abs(score["coverage_pct"] - 200 / 3) < 1e-8
            assert abs(score["lower_pct"] - 100 / 3) < 1e-8 and abs(score["upper_pct"] - 200 / 3) < 1e-8
            ref, = score["examples"]
            assert ref["pointer"] == "/messages/1" and ref["trace"] == "otlp:" + affected["traceId"]
            detail = get(base, "trace", id=ref["trace"], call=ref["call"])["tool_reference_issue"]
            assert detail == {"pointer": ref["pointer"], "has_invocation_id": True, "matching_prior_calls": 0}
            assert get(base, "tool-reference-score", user="bob")["affected"] == 1
            card, = get(base, "findings", user="alice", kind="tool_reference")["rows"]
            assert card["evidence"]["detail"]["reference"] == detail
            assert card["evidence"]["call_id"] == ref["call"] and card["evidence"]["trace_id"] == ref["trace"]
            assert "tool history" in card["action"]["title"]
            send_traces(base, export(late, second_failure))
            end = get(base, "tool-reference-score", user="alice")
            assert (end["clear"], end["affected"], end["unresolved"]) == (0, 1, 2), end
        with sqlite3.connect(db) as sql:
            sql.execute("DROP TABLE tool_reference_scores")
            sql.execute("DELETE FROM observations WHERE kind='tool_reference'")
            sql.execute("PRAGMA user_version=35")
        with server(binary, db, "--capture-content", "--retain-days", "365") as base:
            assert get(base, "tool-reference-score", user="alice") == end
            assert len(get(base, "findings", user="alice", kind="tool_reference")["rows"]) == 1
        with server(binary, Path(tmp) / "redacted.sqlite", "--retain-days", "365") as base:
            send_traces(base, export(*spans))
            score = get(base, "tool-reference-score")
            assert score["pass_pct"] is None and not score["examples"], score
    print("Verified trace counts, missing evidence, scoped witnesses, retries, late spans, redaction and schema 35 recovery.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
