"""Export the four saved first-token and proposal experiments as OTLP/HTTP JSON."""
import argparse
import json
from datetime import datetime
from hashlib import sha256
from pathlib import Path
from urllib.request import Request, urlopen

FOLDER = Path(__file__).parent
ATTRIBUTE = "nexusweave.prefill.probe"


def inputs(record):
    metadata = record["extra"]["metadata"]
    raw = metadata["nexusweave.request.body"]
    messages = json.loads(raw)["messages"]
    return {"gen_ai.operation.name": "chat", "gen_ai.request.model": metadata["ls_model_name"],
            "nexusweave.request.body": raw, "nexusweave.context.semantics": metadata["nexusweave.context.semantics"],
            "gen_ai.input.messages": [{"role": m["role"], "parts": [{"type": "text", "content": m["content"]}]} for m in messages]}


def export(records, probes):
    spans = []
    for record in records:
        metadata = record["extra"]["metadata"]
        attrs = {**inputs(record), ATTRIBUTE: probes[record["id"]],
                 "nexusweave.shadow_probes": metadata["nexusweave.shadow_probes"],
                 "nexusweave.organization.id": metadata["organization_id"], "nexusweave.user.id": metadata["user_id"],
                 "gen_ai.conversation.id": metadata["thread_id"]}
        attributes = [{"key": k, "value": {"stringValue": v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)}}
                      for k, v in attrs.items()]
        trace = sha256(("nexusweave-prefill/" + record["id"]).encode()).hexdigest()[:32]
        timestamp = str(int(datetime.fromisoformat(record["start_time"].replace("Z", "+00:00")).timestamp()) * 10**9)
        spans.append({"traceId": trace, "spanId": trace[:16], "name": record["name"], "startTimeUnixNano": timestamp,
                      "endTimeUnixNano": timestamp, "attributes": attributes})
    return {"resourceSpans": [{"resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "routing-prefill"}}]},
                               "scopeSpans": [{"spans": spans}]}]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", help="POST the saved measurements to this traces endpoint. Defaults to stdout.")
    parser.add_argument("--prepare", metavar="DIRECTORY", type=Path, help="Write a preflight request and edits per case for probe-prefill.")
    args = parser.parse_args()
    records = json.loads((FOLDER / "shadow-cohort.json").read_text())
    if args.prepare:
        args.prepare.mkdir(parents=True, exist_ok=True)
        for record in records:
            report, = record["extra"]["metadata"]["nexusweave.shadow_probes"]
            for suffix, value in [("request", {"version": 1, "attributes": inputs(record)}),
                                  ("edits", [e["intervention"] for e in report["effects"]])]:
                with (args.prepare / (record["id"] + "-" + suffix + ".json")).open("x") as out:
                    json.dump(value, out, ensure_ascii=False)
    else:
        body = json.dumps(export(records, json.loads((FOLDER / "prefill-cohort.json").read_text())))
        if args.endpoint:
            with urlopen(Request(args.endpoint, body.encode(), {"Content-Type": "application/json"}), timeout=30) as response:
                print(f"Sent {len(records)} recorded experiments (HTTP {response.status}).")
        else:
            print(body)
