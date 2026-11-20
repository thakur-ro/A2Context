"""Send a saved Qwen inference and its controlled replay to a live inspector."""

import argparse
import json
import time
from datetime import datetime
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid4, uuid5


def prepare(conversation_id, *, followup=False, declare_windows=False, report_file=None, archived=False):
    fixtures = Path(__file__).parent / "context-screening" / "replay"
    report_file = report_file or ("report-followup.json" if followup else "report.json")
    report = json.loads((fixtures / report_file).read_text(encoding="utf-8"))
    runs = json.loads((fixtures / "evidence.json").read_text(encoding="utf-8"))
    run = next(run for run in runs if run["id"] == report["location"]["run_id"])
    call = run["calls"][0]
    body = call["recorded_request"]["body"]
    request = json.loads(body)
    inputs = [
        {"role": message["role"], "parts": [{"type": "text", "content": message["content"]}]}
        for message in request["messages"]
    ]
    context_ids = {}
    declarations = []
    for item in call["context"]:
        index = item["evidence"]["message_index"]
        context_ids[item["id"]] = f"input:{index}:0"
        if item["source_id"]:
            declarations.append({
                "anchor": {"field": "gen_ai.input.messages", "message_index": index, "part_index": 0},
                "source_id": item["source_id"],
                "source_version": item["source_version"],
            })
            if declare_windows:
                # Human annotations of the two saved messages. They do not modify the replayed request.
                values = {"journal-policy": 30, "journal-faq": 60}
                declarations[-1]["rule"] = {"key": "return_window_days", "scope": "journal", "value": values[item["source_id"]]}
    # OTLP message parts have their own context IDs. Preserve the source versions, digests and scores.
    screen = call["context_screening"]["report"]
    screen["selection"]["context_ids"] = [context_ids[key] for key in screen["selection"]["context_ids"]]
    for source in screen["sources"]:
        source.update(context_id=context_ids[source["context_id"]], kind="text")
    trace_id, span_id = uuid4().hex, uuid4().hex[:16]
    attributes = {
        "gen_ai.operation.name": "chat",
        "gen_ai.request.model": request["model"],
        "gen_ai.conversation.id": conversation_id,
        "nexusweave.organization.id": run["organization_id"],
        "nexusweave.user.id": run["user_id"],
        "nexusweave.turn.id": call["turn_id"],
        "nexusweave.request.body": body,
        "nexusweave.context_screening": json.dumps(screen),
        "nexusweave.context.semantics": json.dumps({"version": 1, "context": declarations}),
        "gen_ai.input.messages": json.dumps(inputs),
        "gen_ai.output.messages": json.dumps([{
            "role": "assistant",
            "parts": [{"type": "text", "content": report["recorded"]["outcome"]}],
        }]),
    }
    timestamp = time.time_ns()
    if archived:
        key = "nexusweave/journal-coverage/" + run["id"] + "/" + conversation_id
        trace_id, span_id = uuid5(NAMESPACE_URL, key).hex, uuid5(NAMESPACE_URL, key + "/call").hex[:16]
        timestamp = int(datetime.fromisoformat(call["started_at"].replace("Z", "+00:00")).timestamp()) * 10**9

    span = {
        "traceId": trace_id,
        "spanId": span_id,
        "name": "Journal return answer",
        "startTimeUnixNano": str(timestamp if archived else timestamp - 1),
        "endTimeUnixNano": str(timestamp),
        "attributes": [
            {"key": key, "value": {"stringValue": value}} for key, value in attributes.items()
        ],
    }
    report["location"] = {"run_id": f"otlp:{trace_id}", "call_id": span_id, "decision_id": "output:0:0"}
    for variant in report["variants"]:
        intervention = variant["intervention"]
        intervention["context_id"] = context_ids[intervention["context_id"]]
    return {"resourceSpans": [{"scopeSpans": [{"spans": [span]}]}]}, report


def value_report(original, *, followup):
    """Bind the saved numerical edit to the same exported call and source IDs."""
    name = "followup" if followup else "earlier"
    path = Path(__file__).parent / "context-screening" / "replay" / (name + "-45-days.json")
    measured = json.loads(path.read_text())
    if measured["baseline_request_sha256"] != original["baseline_request_sha256"]:
        raise ValueError("The numerical replay belongs to a different request.")
    ids = {(v["intervention"]["pointer"], v["captured_context_sha256"]): v["intervention"]["context_id"]
           for v in original["variants"]}
    measured["location"] = original["location"]
    for arm in measured["variants"]:
        arm["intervention"]["context_id"] = ids[arm["intervention"]["pointer"], arm["captured_context_sha256"]]
    return measured


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040", help="Inspector base URL.")
    parser.add_argument("--conversation-id", default="journal-live-replay")
    parser.add_argument("--followup", action="store_true", help="Send the follow-up question and its four saved Qwen conditions.")
    parser.add_argument("--compare-values", action="store_true", help="Also attach the replay that changes only the day count to 45.")
    parser.add_argument("--declare-windows", action="store_true", help="Annotate the two recorded journal rules with their 30- and 60-day values.")
    args = parser.parse_args()
    base_url = args.url.rstrip("/")
    trace, report = prepare(args.conversation_id, followup=args.followup, declare_windows=args.declare_windows)
    posts = [("/v1/traces", trace), ("/api/replay-reports", report)]
    if args.compare_values:
        posts.append(("/api/replay-reports", value_report(report, followup=args.followup)))
    for path, payload in posts:
        request = Request(
            base_url + path, json.dumps(payload).encode(), {"Content-Type": "application/json"},
        )
        with urlopen(request, timeout=10):
            pass
    location = report["location"]
    if args.declare_windows:
        params = {"trace": location["run_id"], "call": location["call_id"]}
        with urlopen(base_url + "/api/sidecar/findings?" + urlencode({**params, "kind": "conflict"}), timeout=10) as response:
            card, = json.load(response)["rows"]
        print(base_url + "/?" + urlencode({"recommendation": card["id"], **params}))
        return
    changed = next(v["intervention"]["context_id"] for v in report["variants"] if v["difference_pp"] != 0)
    query = urlencode({
        "v": 1, "view": "decisions", "run": location["run_id"],
        "call": location["call_id"], "decision": location["decision_id"],
        "replay": report["id"], "variant": changed,
    })
    print(f"{base_url}/#{query}")


if __name__ == "__main__":
    main()
