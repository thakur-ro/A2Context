"""Discover shared prompt identifiers and optionally replay only the matched text on a loopback model."""

import argparse
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import random
import re
import sqlite3
import string
import sys
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts"))
from check_sidecar import send

TOKEN = re.compile(r"(?<![\w-])[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)+(?![\w-])")
URL = re.compile(r"https?://[^\s<>]+", re.I)


def shared_lines(current, earlier):
    """Return character ranges of whole matching lines at each end."""
    left = current.splitlines(keepends=True)
    right = earlier.splitlines(keepends=True)
    front = 0
    while front < min(len(left), len(right)) and left[front] == right[front]:
        front += 1
    back = 0
    while back < min(len(left), len(right)) - front and left[-1 - back] == right[-1 - back]:
        back += 1
    return sum(map(len, left[:front])), len(current) - sum(len(left[-1 - i]) for i in range(back))


def byte_range(text, start, end):
    return [len(text[:start].encode()), len(text[:end].encode())]


def eligible(call):
    decisions = call["decisions"]
    if (
        len(decisions) != 1
        or decisions[0]["kind"] != "model_output"
        or not isinstance(decisions[0]["outcome"], str)
    ):
        return None
    raw = call.get("recorded_request")
    if not raw:
        return None
    body = json.loads(raw["body"])
    messages = body.get("messages")
    if body.get("tools") or body.get("functions") or not messages:
        return None
    if not all(
        m.get("role") in ("user", "system", "developer", "assistant")
        and isinstance(m.get("content"), str)
        and not m.get("tool_calls")
        for m in messages
    ):
        return None
    return body


def select(body, previous, answer):
    records = {}
    for token in TOKEN.finditer(answer):
        key = token.group()
        if (
            len(key) <= 128
            and any(c.isdigit() for c in key)
            and any(c.isalpha() for c in key)
            and key not in records
        ):
            records[key] = {"output_bytes": byte_range(answer, *token.span()), "locations": []}
    if not records:
        return {}
    # An occurrence outside the shared material disqualifies a token even if it appears in both.
    disqualified = {k for k in records if any(k in url for url in URL.findall(answer))}
    for i, (msg, prior) in enumerate(zip(body["messages"], previous["messages"])):
        text = msg["content"]
        front, back = shared_lines(text, prior["content"])
        urls = URL.findall(text)
        for match in TOKEN.finditer(text):
            key = match.group()
            if key not in records:
                continue
            if (match.end() > front and match.start() < back) or any(key in url for url in urls):
                disqualified.add(key)
                continue
            # The suffix may start at a different byte in the earlier input.
            shift = len(prior["content"]) - len(text) if match.start() >= back else 0
            a, b = match.span()
            records[key]["locations"].append(
                {
                    "message": i,
                    "bytes": byte_range(text, a, b),
                    "prior_bytes": byte_range(prior["content"], a + shift, b + shift),
                }
            )
    return dict([(k, r) for k, r in records.items() if k not in disqualified and r["locations"]][:3])


def variant(body, call_id, matches):
    rng = random.Random(call_id)
    original = json.dumps(body, ensure_ascii=False)
    aliases = {}
    for key in matches:
        while True:
            alias = "".join(
                rng.choice(string.ascii_lowercase)
                if c.islower()
                else rng.choice(string.ascii_uppercase)
                if c.isupper()
                else rng.choice(string.digits)
                if c.isdigit()
                else c
                for c in key
            )
            if alias not in original and alias not in aliases.values():
                aliases[key] = alias
                break
    edited = deepcopy(body)
    for m in edited["messages"]:
        m["content"] = TOKEN.sub(lambda tok: aliases.get(tok.group(), tok.group()), m["content"])
    return aliases, edited


def final_calls(folder):
    with sqlite3.connect((folder / "sidecar.sqlite").resolve().as_uri() + "?mode=ro", uri=True) as db:
        for workflow, raw in db.execute("SELECT workflow,run FROM traces WHERE run IS NOT NULL"):
            run = json.loads(raw)
            call = max(run["calls"], key=lambda c: (c["started_at"], c["id"]))
            yield workflow, run, call


def experiment(folder, endpoint):
    history = {}
    for workflow, run, call in sorted(
        final_calls(folder), key=lambda row: (row[2]["started_at"], row[2]["id"])
    ):
        row = {
            "workflow": workflow,
            "session": run["conversation_id"],
            "trace": run["id"],
            "call": call["id"],
        }
        body = eligible(call)
        if body is None or not run["conversation_id"]:
            yield dict(row, status="unassessed")
            continue
        scope = (
            workflow,
            run["source"],
            run["organization_id"],
            run["user_id"],
            call["model"],
            tuple(m["role"] for m in body["messages"]),
        )
        prior = max(
            (
                r
                for r in history.get(scope, [])
                if r[0] < call["started_at"] and r[1] != run["conversation_id"]
            ),
            key=lambda r: r[0],
            default=None,
        )
        history.setdefault(scope, []).append(
            (call["ended_at"], run["conversation_id"], run["id"], call["id"], body)
        )
        if not prior or body["messages"] == prior[4]["messages"]:
            yield dict(row, status="no_comparison")
            continue
        matches = select(body, prior[4], call["decisions"][0]["outcome"])
        if not matches:
            yield dict(row, status="no_match")
            continue
        aliases, edited = variant(body, call["id"], matches)
        row.update(
            status="candidate",
            prior_trace=prior[2],
            prior_call=prior[3],
            matches=matches,
            aliases=aliases,
            captured_sha256=sha256(call["recorded_request"]["body"].encode()).hexdigest(),
        )
        yield row
        if not endpoint:
            continue
        for seed in (11, 29, 47):
            arms = ["original", "repeat", "edit"]
            random.Random(f"{call['id']}/{seed}").shuffle(arms)
            trial = {"trace": run["id"], "call": call["id"], "seed": seed, "order": arms}
            for arm in arms:
                request = dict(
                    edited if arm == "edit" else body, seed=seed, temperature=0.5, max_tokens=512
                )
                try:
                    status, data = send(endpoint, "/v1/chat/completions", request, expected=None)
                    result = {"http": status}
                    if status == 200:
                        text = data["choices"][0]["message"].get("content") or ""
                        terms = {t.group() for t in TOKEN.finditer(text)}
                        result.update(
                            original=sorted(terms.intersection(aliases)),
                            alias=sorted(terms.intersection(aliases.values())),
                            answer_sha256=sha256(text.encode()).hexdigest(),
                        )
                except (URLError, TimeoutError) as exc:
                    result = {"error": str(exc)}
                trial[arm] = result
            yield trial


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("captures", nargs="+", type=Path)
    parser.add_argument(
        "--model-port", type=int, help="Generate three arms at three seeds for every candidate"
    )
    args = parser.parse_args()
    endpoint = f"http://127.0.0.1:{args.model_port}" if args.model_port else None
    for directory in args.captures:
        for row in experiment(directory, endpoint):
            print(json.dumps(row), flush=True)
