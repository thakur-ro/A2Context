"""Import seven saved tool proposals and their measured source masks over OTLP/HTTP."""

import argparse
from datetime import datetime
import json
from pathlib import Path
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

FIXTURES = Path(__file__).parent / "runtime-comparison/visibility-routing-cohort"


def measurement(run, report):
    call, = run["calls"]
    decision, = call["decisions"]
    raw = call["recorded_request"]["body"]
    tools = json.loads(raw)["tools"]
    sources, annotations, ids = [], [], {}
    for source in call["context"]:
        key = source["id"]
        identity = {k: source[k] for k in ("source_id", "source_version")}
        if source["kind"] == "tool_definition":
            part = source["evidence"]["part_index"]
            annotations.append(dict(identity, anchor={"field":"gen_ai.tool.definitions","part_index":part}))
            ids[key] = f"tool_definition:{part}"
        else:
            sources.append(dict(identity, id=key, kind=source["kind"], role=source["role"],
                                request_fragment=source["request_fragment"]))
            ids[key] = f"request-source:{key}"
    trace = uuid5(NAMESPACE_URL, "nexusweave/tool-masks/" + run["id"]).hex
    span = trace[:16]
    fields = {
        "gen_ai.operation.name":"chat", "gen_ai.request.model":call["model"],
        "gen_ai.conversation.id":run["conversation_id"], "nexusweave.turn.id":call["turn_id"],
        "nexusweave.organization.id":run["organization_id"], "nexusweave.user.id":run["user_id"],
        "nexusweave.request.body":raw, "gen_ai.tool.definitions":tools,
        "gen_ai.output.messages":[{"role":"assistant","parts":[{
            "type":"tool_call","id":"proposal","name":decision["label"],"arguments":decision["outcome"]}]}],
        "nexusweave.context.semantics":{"version":1,"context":annotations,"request_sources":sources},
    }
    time = str(int(datetime.fromisoformat(call["started_at"]).timestamp()) * 10**9)
    inference = {
        "traceId":trace,"spanId":span,"name":"Retail tool routing","startTimeUnixNano":time,"endTimeUnixNano":time,
        "attributes":[{"key":k,"value":{"stringValue":v if isinstance(v,str) else json.dumps(v)}} for k,v in fields.items()],
    }
    resource = {"attributes":[{"key":"service.name","value":{"stringValue":"retail-tool-masks"}}]}
    export = {"resourceSpans":[{"resource":resource,"scopeSpans":[{"spans":[inference]}]}]}
    report = dict(report, location={"run_id":"otlp:" + trace,"call_id":span,"decision_id":"output:0:0"})
    report["sources"] = [dict(row, context_id=ids[row["context_id"]]) for row in report["sources"]]
    for key in ("pairs", "continuations"):
        report[key] = [dict(row, context_ids=[ids[i] for i in row["context_ids"]]) for row in report[key]]
    return export, report


def samples():
    reports = {r["location"]["run_id"]:r for r in json.loads((FIXTURES / "reports.json").read_text())}
    for run in json.loads((FIXTURES / "evidence.json").read_text()):
        yield measurement(run, reports[run["id"]])


def send(origin, path, payload):
    request = Request(origin + path, json.dumps(payload).encode(), {"Content-Type":"application/json"})
    with urlopen(request, timeout=30) as response:
        response.read()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    origin = parser.parse_args().url.rstrip("/")
    for export, report in samples():
        send(origin, "/v1/traces", export)
        send(origin, "/api/activation-reports", report)
    print(origin + "/?workflow=retail-tool-masks")
