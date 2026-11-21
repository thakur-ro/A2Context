"""Score APPROVE versus DENY, then generate freely with REVIEW also available."""
import argparse
import json
from pathlib import Path

from replicate_local import MODEL
from propose_qwen import load_runtime, propose
from score_pending_edits import score_request

SYSTEM = ("Evaluate this warranty request using the supplied policy and inspection. A verified exception authorizes approval. "
          "If eligibility cannot be determined, return REVIEW. Reply with exactly one label: APPROVE, REVIEW, or DENY.")
POLICY = "Approve factory faults. Deny accidental damage."
INSPECTIONS = {"fault": "Inspection found a factory fault. No exception is recorded.",
               "accident": "Inspection found accidental damage. No exception is recorded.",
               "pending": "The inspection is pending. No exception is recorded.",
               "exception": "Inspection found accidental damage. A verified exception authorizes approval."}
ABSENT = "No information is available from this source."


def assemble(inspection, order, edit=None):
    texts = {"policy": POLICY, "inspection": inspection}
    messages = [{"role": "system", "content": SYSTEM}]
    sources = []
    for index, source in enumerate(order, 1):
        prefix = source.capitalize() + ": "
        text = prefix + (ABSENT if source == edit else texts[source])
        messages.append({"role": "user", "content": text})
        sources.append({"id": source, "source_id": source, "source_version": "1", "role": "user",
                        "request_fragment": {"pointer": f"/messages/{index}/content",
                                             "text_range": {"start_byte": len(prefix.encode()), "end_byte": len(text.encode())}}})
    messages.append({"role": "user", "content": "What is the disposition of this warranty request?"})
    body = {"model": MODEL, "messages": messages, "temperature": 0, "max_tokens": 16}
    return {"version": 1, "attributes": {"gen_ai.request.model": MODEL,
            "a2context.request.body": json.dumps(body, separators=(",", ":")),
            "a2context.context.semantics": {"version": 1, "request_sources": sources}}}


def run(binary, snapshot, revision):
    cases = []
    example = None
    for name, inspection in INSPECTIONS.items():
        for order in [("policy", "inspection"), ("inspection", "policy")]:
            request = assemble(inspection, order)
            report = score_request(binary, snapshot, revision, request, name + "-" + order[0], ("APPROVE", "DENY"), ABSENT)
            if name == "accident" and order[0] == "policy":
                example = {"request": request, "candidate": report}
            cases.append({"scenario": name, "order": order, "baseline_log_odds": report["baseline_log_odds"],
                          "conditions": report["scores"]["conditions"],
                          "edited_log_odds": {e["source_id"]: e["log_odds"] for e in report["effects"]}})
    # Freeze every score before generating any baseline or edited answer.
    runtime = load_runtime(snapshot)
    with runtime.torch.inference_mode():
        for case in cases:
            case["answers"] = {}
            for source in [None, *case["order"]]:
                request = assemble(INSPECTIONS[case["scenario"]], case["order"], source)
                body = json.loads(request["attributes"]["a2context.request.body"])
                case["answers"][source or "baseline"] = propose(runtime, body, MODEL)["outcome"]
    return {"model": MODEL, "revision": revision, "runtime": runtime.identity, "cases": cases, "example": example}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args.binary.resolve(), args.snapshot, args.revision)
    with args.output.open("x") as handle:
        json.dump(result, handle, indent=2)
        handle.write("\n")
