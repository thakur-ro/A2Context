"""Load the eight saved calendar experiments as OpenTelemetry conversations."""
import argparse
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

CASES = Path(__file__).parent / "context-screening/replay/prefix-transfer/cases.json"
ORG = "calendar-prefix-study"


def trace_for(case, index):
    envelope = case["envelope"]
    fields = envelope["request"]["attributes"].copy()
    body = json.loads(fields["nexusweave.request.body"])
    shadow = envelope["shadow"]
    fields.update({
        "gen_ai.operation.name": "chat", "gen_ai.conversation.id": "Booking " + str(index),
        "nexusweave.organization.id": ORG, "nexusweave.user.id": "calendar-evaluation",
        "nexusweave.candidate_probes": [envelope["candidate"]], "nexusweave.shadow_probes": [shadow],
        "gen_ai.input.messages": [{"role":m["role"],"parts":[{"type":"text","content":m["content"]}]} for m in body["messages"]],
    })
    original = shadow["baseline"][0].get("choice")
    if original is not None:
        fields["gen_ai.output.messages"] = [{"role":"assistant","parts":[{"type":"text","content":original["outcome"]}]}]
    trace = uuid5(NAMESPACE_URL, ORG + "/" + case["id"]).hex
    when = str((1795172400 + index) * 10**9)
    span = {"traceId":trace, "spanId":trace[:16], "name":"Room booking", "startTimeUnixNano":when, "endTimeUnixNano":when,
            "attributes":[{"key":k,"value":{"stringValue":v if isinstance(v,str) else json.dumps(v)}} for k,v in fields.items()]}
    return {"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"calendar-booking"}}]},
                              "scopeSpans":[{"spans":[span]}]}]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    base = parser.parse_args().url.rstrip("/")
    for i, case in enumerate(json.loads(CASES.read_text()), 1):
        request = Request(base + "/v1/traces", json.dumps(trace_for(case, i)).encode(), {"Content-Type":"application/json"})
        with urlopen(request, timeout=30) as response:
            response.read()
    with urlopen(base + "/api/sidecar/prefix-groups?" + urlencode({"org":ORG}), timeout=20) as response:
        groups = json.load(response)["rows"]
    if len(groups) != 1 or groups[0]["conversations"] != 8:
        raise ValueError("Expected eight captured calendar conversations. Run the sidecar with --capture-content.")
    print(base + "/?" + urlencode({"org":ORG}))
