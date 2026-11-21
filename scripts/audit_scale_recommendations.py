"""Audit exact duplicate evidence behind ranked cards on an imported study."""

import argparse
from collections import Counter
import json
from pathlib import Path
import sqlite3
from urllib.parse import urlencode, urlsplit
from urllib.request import urlopen


def api(base, endpoint, **query):
    with urlopen(f"{base}/api/sidecar/{endpoint}?{urlencode(query)}", timeout=60) as response:
        return json.load(response)


def baseline(db, workflow, ordering):
    order = {
        "frequency": "conversations DESC, mentions DESC, latest DESC",
        "recency": "latest DESC, conversations DESC, mentions DESC",
    }[ordering]
    query = f"""
        WITH scoped AS (
            SELECT o.*, t.thread_key FROM observations o
            JOIN traces t ON t.id=o.trace_id WHERE t.workflow=?
        ), ranked AS (
            SELECT group_id,count(DISTINCT thread_key) AS conversations,
                count(*) AS mentions,max(observed) AS latest
            FROM scoped GROUP BY group_id ORDER BY {order},group_id LIMIT 5
        )
        SELECT s.group_id,s.kind,g.conversations,g.mentions,s.trace_id,s.call_id,s.detail
        FROM ranked g JOIN scoped s USING(group_id)
        WHERE (s.trace_id,s.n) = (
            SELECT t.trace_id,t.n FROM scoped t WHERE t.group_id=g.group_id
                ORDER BY t.observed DESC,t.trace_id,t.call_id,t.n LIMIT 1
        ) ORDER BY {order},s.group_id
    """
    return [
        {"id": group, "kind": kind, "threads": conversations, "occurrences": occurrences,
         "evidence": {"trace_id": trace, "call_id": call, "detail": json.loads(detail)}}
        for group, kind, conversations, occurrences, trace, call, detail in db.execute(query, (workflow,))
    ]


def audit_card(base, card, traces):
    evidence = card["evidence"]
    answer = {
        "id": card["id"], "kind": card["kind"], "conversations": card["threads"],
        "observations": card["occurrences"], "trace": evidence["trace_id"], "call": evidence["call_id"],
    }
    if card["kind"] != "duplicate":
        return answer
    trace = evidence["trace_id"]
    if trace not in traces:
        traces[trace] = api(base, "trace", id=trace)["trace"]
    call = next(call for call in traces[trace]["calls"] if call["id"] == evidence["call_id"])
    inputs = {source["id"]: source for source in call["context"]}
    sources = [inputs[source["id"]] for source in evidence["detail"]["sources"]]
    text = sources[0]["text"]
    if text != sources[1]["text"]:
        raise ValueError(f"Source text differs for duplicate card {card['id']}.")
    tool_messages = {source["evidence"].get("message_index") for source in inputs.values()
                     if source["role"] == "assistant" and source["kind"] == "tool_call"}
    marker = text.rstrip() == "(no content)" and all(
        s["role"] == "assistant" and s["evidence"].get("message_index") in tool_messages for s in sources
    )
    answer.update({
        "characters": len(text), "copies": evidence["detail"]["copies"],
        "source_roles": [source["role"] for source in sources],
        "evidence_class": (
            "tool_message_placeholder" if marker else
            "unattributed_dialogue" if evidence["detail"]["dialogue_only"] else "other_exact_repeat"
        ),
    })
    return answer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", required=True)
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("workflows", nargs="+")
    args = parser.parse_args()
    origin = urlsplit(args.sidecar)
    if origin.scheme != "http" or origin.hostname not in {"127.0.0.1", "localhost"} or origin.path not in {"", "/"}:
        parser.error("--sidecar must be a loopback HTTP origin")
    base = args.sidecar.rstrip("/")
    report = {}
    with sqlite3.connect(args.db.resolve().as_uri() + "?mode=ro", uri=True) as db:
        db.execute("BEGIN")
        for workflow in args.workflows:
            trace_cache = {}
            cards = {"a2context": api(base, "findings", workflow=workflow)["rows"][:5]}
            for sort in ("frequency", "recency"):
                cards[sort] = baseline(db, workflow, sort)
            report[workflow] = {
                "coverage": api(base, "health", workflow=workflow)["coverage"],
                "rankings": {name: [audit_card(base, card, trace_cache) for card in rows]
                             for name, rows in cards.items()},
            }
            counts = Counter(card.get("evidence_class", card["kind"])
                             for card in report[workflow]["rankings"]["a2context"])
            print(workflow, dict(counts))
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
