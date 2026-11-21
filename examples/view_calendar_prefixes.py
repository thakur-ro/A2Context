"""Load saved calendar experiments as scored or checked OpenTelemetry conversations."""
import argparse
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import NAMESPACE_URL, uuid5

CASES = Path(__file__).parent / "context-screening/replay/prefix-transfer/cases.json"
BLOCKS = CASES.parent.parent / "prefix-blocks/results.json"
ORG = "calendar-prefix-study"


def trace_for(case, index, *, org=ORG, scores_only=False, reference=False, block_probe=None):
    envelope = case["envelope"]
    fields = envelope["request"]["attributes"].copy()
    body = json.loads(fields["a2context.request.body"])
    fields.update({
        "gen_ai.operation.name": "chat", "gen_ai.conversation.id": "Booking " + str(index),
        "a2context.organization.id": org, "a2context.user.id": "calendar-evaluation",
        "gen_ai.input.messages": [{"role":m["role"],"parts":[{"type":"text","content":m["content"]}]} for m in body["messages"]],
    })
    candidate = envelope["candidate"].copy()
    if block_probe is not None:
        candidate["block_probe"] = block_probe
    if reference:
        candidate["reference_target"] = {
            "set":"original-booking-duration-v1", "candidate_id":str(case["minutes"][case["requested"]])}
    fields["a2context.candidate_probes"] = [candidate]
    if not scores_only:
        shadow = envelope["shadow"]
        fields["a2context.shadow_probes"] = [shadow]
        original = shadow["baseline"][0].get("choice")
        if original is not None:
            fields["gen_ai.output.messages"] = [{"role":"assistant","parts":[{"type":"text","content":original["outcome"]}]}]
    trace = uuid5(NAMESPACE_URL, org + "/" + case["id"]).hex
    when = str((1795172400 + index) * 10**9)
    span = {"traceId":trace, "spanId":trace[:16], "name":"Room booking", "startTimeUnixNano":when, "endTimeUnixNano":when,
            "attributes":[{"key":k,"value":{"stringValue":v if isinstance(v,str) else json.dumps(v)}} for k,v in fields.items()]}
    return {"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"calendar-booking"}}]},
                              "scopeSpans":[{"spans":[span]}]}]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    parser.add_argument("--scores-only", action="store_true", help="Withhold all generated outputs and shadow trials.")
    parser.add_argument("--reference", action="store_true", help="Attach the expected duration from the original booking as evaluation metadata.")
    parser.add_argument("--org", help="Organization for this copy of the study.")
    parser.add_argument("--indirect", action="store_true", help="Load the study with activity paraphrases.")
    args = parser.parse_args()
    base = args.url.rstrip("/")
    org = args.org or (("activity-prefix-study" if args.indirect else ORG) + ("-scored" if args.scores_only else ""))
    if args.reference and not args.org:
        org += "-reference"
    path = CASES.parent.parent / "prefix-indirect/cases.json" if args.indirect else CASES
    cases = json.loads(path.read_text())
    blocks = json.loads(BLOCKS.read_text())["reports"] if args.indirect else {}
    for i, case in enumerate(cases, 1):
        trace = trace_for(case, i, org=org, scores_only=args.scores_only, reference=args.reference,
                          block_probe=blocks.get(case["id"]))
        request = Request(base + "/v1/traces", json.dumps(trace).encode(), {"Content-Type":"application/json"})
        with urlopen(request, timeout=30) as response:
            response.read()
    with urlopen(base + "/api/sidecar/prefix-groups?" + urlencode({"org":org}), timeout=20) as response:
        groups = json.load(response)["rows"]
    if len(groups) != 1 or groups[0]["conversations"] != len(cases):
        raise ValueError("Missing study conversations. Run the sidecar with --capture-content.")
    print(base + "/?" + urlencode({"org":org}))
