"""Smoke-test the shipping image from a producer in the same network namespace."""
import argparse
from pathlib import Path
import subprocess
import sys
import time
from urllib.error import URLError
from uuid import uuid4

from check_sidecar import case_span, export, get, send_traces

BASE = "http://127.0.0.1:8040"


def probe(phase):
    for attempt in range(60):
        try:
            health = get(BASE, "health")
            break
        except URLError:
            if attempt == 59:
                raise
            time.sleep(0.25)
    captured = phase == "captured"
    assert not health["demo"] and health["content_capture"] == captured, health
    if phase != "reopen":
        assert health["coverage"]["traces"] == 0, health
        batch = export(case_span("shipping-image", "conflict"))
        send_traces(BASE, batch)
        send_traces(BASE, batch)
    health = get(BASE, "health")
    assert health["accepted_batches"] == (0 if phase == "reopen" else 2), health
    assert health["queue_drops"] == health["rejected_batches"] == 0, health
    assert health["coverage"] == {"traces": 1, "inference_spans": 1, "with_inputs": int(captured),
                                  "with_request": 0, "with_rules": int(captured)}, health
    if captured:
        card, = get(BASE, "findings", kind="conflict")["rows"]
        assert card["evidence"]["trace_id"] == "otlp:" + batch["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"]
        print("PASS enabled capture · synthetic conflict links to the exported trace", flush=True)
    else:
        assert not get(BASE, "findings")["rows"]
        print("PASS", phase, "· one retained inference span, default redaction, zero dropped batches", flush=True)


def check_image(image, client):
    name = "a2context-check-" + uuid4().hex
    scripts = Path(__file__).resolve().parent

    def docker(*args):
        subprocess.run(["docker", *args], check=True)

    docker("volume", "create", name)
    try:
        for phase in ("send", "reopen", "captured"):
            flags = ["--db", "/state/captured.sqlite", "--capture-content"] if phase == "captured" else []
            docker("run", "--detach", "--name", name, "--network", "none", "--read-only",
                   "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--cpus", "1", "--memory", "512m",
                   "--mount", f"type=volume,src={name},dst=/state", image, *flags)
            try:
                docker("run", "--rm", "--network", "container:" + name, "--read-only",
                       "--mount", f"type=bind,src={scripts},dst=/checks,readonly", client,
                       "python", "-B", "/checks/check_image.py", "--probe", phase)
            except subprocess.CalledProcessError:
                subprocess.run(["docker", "logs", "--tail", "30", name], check=False)
                raise
            finally:
                docker("rm", "--force", name)
    finally:
        docker("volume", "rm", name)
    print("PASS shipping image · loopback export, retry, container replacement and conflict evidence", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="a2context:local")
    parser.add_argument("--client", default="python:3.13-alpine")
    parser.add_argument("--probe", choices=("send", "reopen", "captured"), help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not __debug__:
        sys.exit("Run with Python assertions enabled.")
    if args.probe:
        probe(args.probe)
    else:
        check_image(args.image, args.client)
