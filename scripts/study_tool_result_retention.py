"""Measure whether a known tool result was in the request before the same tool was proposed again."""

import argparse
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from datetime import datetime
from hashlib import sha256
import json
from pathlib import Path
import re

from pyarrow.parquet import ParquetFile

from import_exgentic_scale import REVISION, imported_trace_id

READ = re.compile(r"^(?:get|show|list|find|search|lookup|read|check)(?:_|$)", re.I)


def parts(messages, kind):
    for msg_index, message in enumerate(messages):
        for part_index, part in enumerate(message.get("parts") or []):
            if part.get("type") == kind and part.get("id"):
                yield msg_index, part_index, part


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def arguments(part):
    value = part.get("arguments")
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except ValueError:
            pass
    return canonical(value)


def has_error_field(value, depth=0):
    """Recognize explicit error fields in the published JSON tool result envelopes."""
    if depth == 8:
        return False
    if isinstance(value, str):
        try:
            return has_error_field(json.loads(value), depth + 1)
        except ValueError:
            return False
    if isinstance(value, list):
        return any(has_error_field(item, depth + 1) for item in value)
    if isinstance(value, dict):
        if value.get("error") or value.get("isError") is True:
            return True
        if value.get("type") == "text":
            return has_error_field(value.get("text"), depth + 1)
    return False


@dataclass(frozen=True)
class Result:
    digest: str
    span: str
    location: str
    trace: str
    started_at: str
    has_error_field: bool


def observed_repeats(row):
    """Use the first observation of a result and the most recent resolved proposal."""
    by_lane = defaultdict(list)
    for span in row["spans"]:
        lane = span["attributes"].get("gen_ai.conversation.id")
        if lane and span["start_time"]:
            by_lane[(span["trace_id"], lane)].append(span)
    for spans in by_lane.values():
        proposals = {}
        results = {}
        most_recent = {}
        witnesses = []
        for span in sorted(spans, key=lambda span: (datetime.fromisoformat(span["start_time"]), span["span_id"])):
            started_at = datetime.fromisoformat(span["start_time"])
            attrs = span["attributes"]
            inputs = json.loads(attrs.get("gen_ai.input.messages") or "[]") or []
            if not inputs:
                continue
            captured = {}
            newly_seen = []
            for message, part, response in parts(inputs, "tool_call_response"):
                tool_id = response["id"]
                value = response.get("result", response.get("response", response.get("content")))
                captured[tool_id] = value
                if tool_id in proposals and tool_id not in results and value is not None:
                    result = Result(
                        sha256(canonical(value).encode()).hexdigest(),
                        span["span_id"], f"input:{message}:{part}", imported_trace_id(row, span), span["start_time"],
                        has_error_field(value),
                    )
                    results[tool_id] = result
                    newly_seen.append(tool_id)
            outputs = json.loads(attrs.get("gen_ai.output.messages") or "[]") or []
            for message, part, output in parts(outputs, "tool_call"):
                tool_id = output["id"]
                name = output.get("name")
                if not name or output.get("arguments") is None or tool_id in proposals:
                    continue
                signature = (name, arguments(output))
                earlier = most_recent.get(signature)
                if earlier and datetime.fromisoformat(results[earlier].started_at) < started_at:
                    previous = results[earlier]
                    if earlier in captured:
                        retention = "present"
                    elif any(value is not None and sha256(canonical(value).encode()).hexdigest() == previous.digest
                             for value in captured.values()):
                        retention = "same_payload_other_id"
                    else:
                        retention = "absent"
                    witnesses.append({
                        "tool": name,
                        "read_like_name": bool(READ.match(name.rsplit("__", 1)[-1])),
                        "retention": retention,
                        "previous_result": {**asdict(previous), "invocation": earlier},
                        "repeated_proposal": {
                            "span": span["span_id"], "trace": imported_trace_id(row, span),
                            "location": f"output:{message}:{part}", "invocation": tool_id,
                        },
                    })
                proposals[tool_id] = signature
            # Newly witnessed results qualify as prior evidence on the next recorded request.
            for tool_id in newly_seen:
                most_recent[proposals[tool_id]] = tool_id
        for witness in witnesses:
            repeated = results.get(witness["repeated_proposal"]["invocation"])
            original = witness["previous_result"]
            witness["later_result"] = asdict(repeated) if repeated else None
            witness["result_comparison"] = (
                "unobserved" if repeated is None else
                "equal" if repeated.digest == original["digest"] else "different"
            )
            yield witness


def scan(paths):
    counts = defaultdict(Counter)
    cohorts = defaultdict(Counter)
    samples = defaultdict(dict)
    absent_by_tool = defaultdict(Counter)
    for path in paths:
        columns = ["benchmark", "session_id", "harness", "success", "spans"]
        for batch in ParquetFile(path).iter_batches(batch_size=1, columns=columns):
            row = batch.to_pylist()[0]
            workflow = row["benchmark"]
            stats = counts[workflow]
            stats["sessions"] += 1
            stats["spans"] += len(row["spans"])
            found = set()
            for event in observed_repeats(row):
                retention, comparison = event["retention"], event["result_comparison"]
                label = f"{retention}/{comparison}"
                stats["observations"] += 1
                stats[label] += 1
                cohorts[(workflow, row["harness"])][label] += 1
                found.add(label)
                later = event["later_result"]
                if later is not None:
                    previous_error = event["previous_result"]["has_error_field"]
                    later_error = later["has_error_field"]
                    if previous_error or later_error:
                        stats[f"explicit_error/{label}/{int(previous_error)}->{int(later_error)}"] += 1
                if event["read_like_name"]:
                    if retention == "absent":
                        pattern = (workflow, row["harness"], event["tool"], comparison)
                        absent_by_tool[pattern][row["session_id"]] += 1
                    stats[f"read_like/{label}"] += 1
                    if comparison != "unobserved":
                        # Hold at most one example per session and class.
                        key = (workflow, retention, comparison)
                        bucket = samples[key]
                        digest = sha256(f"{row['session_id']}:{retention}:{comparison}".encode()).hexdigest()
                        bucket.setdefault(digest, {
                            "workflow": workflow, "session": row["session_id"], "harness": row["harness"],
                            "success": row["success"], **event,
                        })
                        if len(bucket) > 5:
                            del bucket[max(bucket)]
            stats["sessions_with_observations"] += bool(found)
            stats.update(f"sessions/{label}" for label in found)
        print(path.name, "scanned", flush=True)
    return {"revision": REVISION, "workflow_counts": dict(counts),
            "harness_counts": [{"workflow": wf, "harness": h, "counts": c} for (wf, h), c in sorted(cohorts.items())],
            "absent_read_like_patterns": [
                {"workflow": workflow, "harness": harness, "tool": tool, "later_result": comparison,
                 "observations": sum(sessions.values()), "sessions": len(sessions)}
                for (workflow, harness, tool, comparison), sessions in sorted(absent_by_tool.items())
            ],
            "examples": [item for key in sorted(samples) for _, item in sorted(samples[key].items())]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("shards", nargs="+", type=Path)
    args = parser.parse_args()
    report = scan(sorted(args.shards))
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
