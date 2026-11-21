"""Exercise conditional reads against a loopback origin seeded with recorded tool values."""

import argparse
from collections import Counter
from contextlib import contextmanager
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
from threading import Thread
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from preview_tool_result_freshness import captured_value
from replay_tool_result_retention import require_loopback
from study_tool_result_retention import canonical


def checksum(payload):
    return sha256(payload).hexdigest()


@contextmanager
def fixture_origin():
    # Values are replaced atomically. A handler serves one immutable revision from the map.
    resources = {}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            state = resources.get(self.path)
            if state is None:
                self.send_error(404)
                return
            revision, payload = state
            tag = '"' + checksum(canonical([self.path, revision]).encode()) + '"'
            if self.headers.get("If-None-Match") == tag:
                self.send_response(304)
                self.send_header("ETag", tag)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("ETag", tag)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        worker = Thread(target=server.serve_forever)
        worker.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}", resources
        finally:
            server.shutdown()
            worker.join()


def fetch(url, tag=None):
    request = Request(url, headers={"If-None-Match": tag} if tag else {})
    try:
        with urlopen(request, timeout=10) as response:
            return response.status, response.headers["ETag"], response.read()
    except HTTPError as error:
        with error:
            if error.code != 304:
                raise
            return error.code, error.headers["ETag"], error.read()


def verified_read(url, cached_payload, cached_tag, trusted_digest):
    # A local digest mismatch disables validation against the damaged copy.
    tag = cached_tag if checksum(cached_payload) == trusted_digest else None
    status, returned_tag, payload = fetch(url, tag)
    if status == 304 and tag is not None and returned_tag == tag and not payload:
        return "reused", cached_payload
    if status != 200:
        raise ValueError("Origin did not return a usable representation.")
    return "fetched", payload


def exercise(origin, resources, case, prior, later):
    key = checksum(canonical([case["session"], case["repeated_proposal"]["invocation"]]).encode())
    first = f"/{key}/first"
    second = f"/{key}/second"
    resources[first] = 1, prior
    resources[second] = 1, later
    _, tag, cached = fetch(origin + first)
    trusted = checksum(cached)
    results = {}

    results["unchanged"] = verified_read(origin + first, cached, tag, trusted)
    results["wrong_scope"] = verified_read(origin + second, cached, tag, trusted)
    results["corrupt_cache"] = verified_read(origin + first, cached + b" ", tag, trusted)
    resources[first] = 2, prior
    results["no_op_write"] = verified_read(origin + first, cached, tag, trusted)

    # Simulate a state write after an independent check and before the conditional read.
    _, tag_after_no_op, after_no_op = fetch(origin + first)
    status, _, _ = fetch(origin + first, tag_after_no_op)
    if status != 304:
        raise ValueError("The origin changed before the scheduled write.")
    resources[first] = 3, later
    results["write_after_check"] = verified_read(origin + first, after_no_op, tag_after_no_op, checksum(after_no_op))

    expected = {
        "unchanged": ("reused", prior),
        "wrong_scope": ("fetched", later),
        "corrupt_cache": ("fetched", prior),
        "no_op_write": ("fetched", prior),
        "write_after_check": ("fetched", later),
    }
    if results != expected:
        raise ValueError(f"Conditional read failed for {case['workflow']}/{case['session']}.")
    return {arm: {"outcome": mode, "served_sha256": checksum(payload)}
            for arm, (mode, payload) in results.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", default="http://127.0.0.1:18058")
    parser.add_argument("--study", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    require_loopback(args.sidecar)
    sidecar = args.sidecar.rstrip("/")
    study = json.loads(args.study.read_text(encoding="utf-8"))
    checks = Counter()
    rows = []
    with fixture_origin() as (origin, resources):
        for case in study["examples"]:
            original = case["previous_result"]
            prior = canonical(captured_value(sidecar, original, original["invocation"])).encode()
            later = canonical(captured_value(sidecar, case["later_result"], case["repeated_proposal"]["invocation"])).encode()
            outcomes = exercise(origin, resources, case, prior, later)
            rows.append({"workflow": case["workflow"], "session": case["session"],
                         "proposal_span": case["repeated_proposal"]["span"], "arms": outcomes})
            checks.update(outcomes.keys())
    report = {"revision": study["revision"], "checks": dict(checks), "examples": rows}
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("Verified conditional HTTP reads", dict(checks), flush=True)


if __name__ == "__main__":
    main()
