"""Load the recorded ALFWorld actions from Who&When Pro into Threads."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
from urllib.parse import urlsplit

from check_sidecar import attrs, send_traces
from study_action_evidence import REVISION, TEXT_SHA256, recorded_steps


def import_observations(archive, sidecar):
    checksum = sha256()
    traces = []
    seen = set()
    with archive.open("rb") as stream:
        for line in stream:
            checksum.update(line)
            row = json.loads(line)
            if row["framework"] != "alfagent" or row["benchmark"] != "alfworld":
                continue
            identity = row["id"]
            if identity in seen:
                raise ValueError("Duplicate ALFWorld trace ID")
            seen.add(identity)
            trace_id = sha256(f"{REVISION}:{identity}".encode()).hexdigest()[:32]
            trajectory = json.loads(row["trajectory"])
            spans = []
            for step in recorded_steps("alfagent", trajectory):
                observation = trajectory[step.index].get("observation")
                if not step.action or not isinstance(observation, str) or not observation:
                    continue
                spans.append({"traceId": trace_id, "spanId": f"{step.index + 1:016x}",
                              "name": "ALFWorld action", "attributes": attrs({
                                  "gen_ai.operation.name": "execute_tool",
                                  "gen_ai.tool.name": step.action,
                                  "gen_ai.tool.call.result": observation,
                                  "gen_ai.conversation.id": identity,
                                  "a2context.organization.id": "whowhen-pro",
                                  "a2context.turn.id": str(step.coordinate),
                              })})
            if spans:
                traces.append(spans)
    if checksum.hexdigest() != TEXT_SHA256:
        raise ValueError("Text archive does not match the pinned release")
    resource = {"attributes": attrs({"service.name": "alfworld"})}
    for start in range(0, len(traces), 16):
        spans = [span for trace in traces[start:start + 16] for span in trace]
        batch = {"resourceSpans": [{"resource": resource, "scopeSpans": [{"spans": spans}]}]}
        send_traces(sidecar, batch)
    return len(traces), sum(map(len, traces))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--sidecar", default="http://127.0.0.1:8040")
    args = parser.parse_args()
    url = urlsplit(args.sidecar)
    if url.scheme != "http" or url.hostname not in {"localhost", "127.0.0.1", "::1"}:
        parser.error("Use a local HTTP sidecar URL")
    traces, events = import_observations(args.archive, args.sidecar.rstrip("/"))
    print(f"Imported {events} recorded actions from {traces} trajectories")


if __name__ == "__main__":
    main()
