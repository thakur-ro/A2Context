"""Replay the deterministic rule fixture and emit the proposed decision as JSON."""
import json
import re
import sys


request = json.load(sys.stdin)
for message in request["messages"]:
    match = re.search(r"(\d+)-day window", message["content"])
    if match:
        json.dump({
            "kind": "tool_call",
            "label": "check_return_eligibility",
            "outcome": {"window_days": int(match[1])},
        }, sys.stdout)
        break
