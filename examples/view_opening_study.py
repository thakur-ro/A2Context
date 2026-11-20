"""Send the eight saved opening-token experiments to a sidecar without running a model."""
import argparse
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from urllib.request import urlopen
from uuid import NAMESPACE_URL, uuid5

ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / "context-screening" / "replay"
sys.path[:0] = [str(FIXTURES), str(ROOT / "public-qa")]
from check_question_holdout import ATTRIBUTE, conditions
from view_passage_edits import post, verified_report

ORG = "journal-opening-study"


def records():
    reference = json.loads((FIXTURES / "case.json").read_text())
    cases = json.loads((FIXTURES / "question-results.json").read_text())["cases"]
    tokens = json.loads((FIXTURES / "held-opening-tokens.json").read_text())
    for index, case in enumerate(cases, 1):
        question, scores, answers = case["question"], case["scores"], case["answers"]
        batch, request, edits = conditions(reference, question)
        original = batch["baseline"]
        trace_id = uuid5(NAMESPACE_URL, ORG + "/" + scores["baseline_request_sha256"]).hex
        span_id = trace_id[:16]
        attrs = request["attributes"]
        attrs.update({
            ATTRIBUTE: {"interventions": edits, "scores": scores},
            "gen_ai.operation.name": "chat",
            "gen_ai.conversation.id": f"Journal question {index}",
            "nexusweave.organization.id": ORG,
            "nexusweave.user.id": "journal-evaluation",
            "gen_ai.input.messages": [
                {"role": m["role"], "parts": [{"type": "text", "content": m["content"]}]}
                for m in original["messages"]
            ],
        })
        contrast = tokens.get(scores["baseline_request_sha256"])
        if contrast:
            attrs[ATTRIBUTE]["token_contrast"] = contrast
        if answers["original"] is not None:
            attrs["gen_ai.output.messages"] = [
                {"role": "assistant", "parts": [{"type": "text", "content": answers["original"]}]}
            ]
        # Fixed demonstration timestamps keep retries identical across invocations.
        when = str((1795172400 + index) * 10**9)
        span = {
            "traceId": trace_id, "spanId": span_id, "name": "Journal answer",
            "startTimeUnixNano": when, "endTimeUnixNano": when,
            "attributes": [
                {"key": key, "value": {"stringValue": value if isinstance(value, str) else json.dumps(value)}}
                for key, value in attrs.items()
            ],
        }
        trace = {"resourceSpans": [{"scopeSpans": [{"spans": [span]}]}]}
        replay = {
            "version": 1, "id": f"journal-opening-{index}", "samples": 1, "runner_id": reference["runner_id"],
            "location": {"run_id": "otlp:" + trace_id, "call_id": span_id, "decision_id": "output:0:0"},
            "request": original, "interventions": edits,
        }
        outcomes = {(): (original, answers["original"])}
        for arm in batch["edits"]:
            outcomes[(arm["context_id"],)] = (arm["request"], answers[arm["source_id"]])
        yield trace, replay, outcomes


def load(base, binary):
    for trace, replay, outcomes in records():
        post(base, "/v1/traces", trace)
        if outcomes[()][1] is not None:
            with TemporaryDirectory() as tmp:
                report = verified_report(binary, Path(tmp), trace, replay, outcomes)
            post(base, "/api/replay-reports", report)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8040")
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nexusweave"))
    args = parser.parse_args()
    base = args.url.rstrip("/")
    load(base, args.binary.resolve())
    with urlopen(base + "/api/sidecar/impact?" + urlencode({"org":ORG}), timeout=20) as response:
        gap = json.load(response)["opening_gap"]
    if not gap or [gap[k] for k in ("affected","tested","scored")] != [3,5,8]:
        raise ValueError("The dashboard counts differ from the eight saved experiments.")
    print(base + "/?" + urlencode({"org":ORG}))
