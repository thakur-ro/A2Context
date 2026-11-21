"""Build a reviewer packet from a finished pilot without changing its ranking."""

from hashlib import sha256
import json
from pathlib import Path
import sqlite3
import sys

from run import questions


def build(directory):
    events = [json.loads(line) for line in (directory / "events.jsonl").read_text().splitlines()]
    starts = [e for e in events if e["event"] == "started"]
    prompts = {f"{directory.name}-{i}": q for i, q in enumerate(questions(directory.name), 1)}
    cards = {}
    for event in events:
        for card in event.get("cards", []):
            cards.setdefault(card["id"], (event["at"], card))
    lines = [f"# {directory.name.capitalize()} workflow review", "",
             f"{len(starts)} sessions. {len(cards)} distinct recommendations. Record your review time in UTC.", ""]
    for when, card in list(cards.values())[:5]:
        evidence = card["evidence"]
        lines.extend([f"## {card['action']['title']}", "", f"First observed {when}.", "",
                      f"Evidence in trace `{evidence['trace_id']}`, call `{evidence['call_id']}`.",
                      "", "Disposition and reason (worth investigating, dismissed, or unreviewed): __________", ""])
    lines.extend(["## Quiet sessions", "", "For each session, was there a concrete problem you would investigate? "
                  "Please cite the message or tool call, or record that you found none.", ""])
    with sqlite3.connect(f"{(directory / 'sidecar.sqlite').as_uri()}?mode=ro", uri=True) as db:
        flagged = {row[0] for row in db.execute("SELECT DISTINCT trace_id FROM observations")}
        quiet = sorted((e for e in starts if e["trace"] not in flagged), key=lambda e: sha256(e["trace"].encode()).digest())
        for e in quiet[:3]:
            name, trace = e["run"], e["trace"]
            row = db.execute("SELECT run FROM traces WHERE id=?", (trace,)).fetchone()
            lines.extend([f"### {name}", "", prompts[name], "", f"Trace `{trace}`. [Application output]({name}.log).", ""])
            output = (directory / f"{name}.log").read_text().strip().splitlines()
            if output:
                last = output[-1]
                lines.extend(["> " + last[:520] + ("…" if len(last) > 520 else ""), ""])
            if row:
                run = json.loads(row[0])
                (directory / f"{name}.json").write_text(json.dumps(run, ensure_ascii=False, indent=2))
                lines.extend([f"[Full recorded requests and responses]({name}.json)", ""])
                for c in sorted(run["calls"], key=lambda c: (c["started_at"], c["id"])):
                    kinds = ", ".join(d["kind"] for d in c["decisions"]) or "no output captured"
                    lines.append(f"- Call `{c['id']}` recorded {kinds}.")
            else:
                lines.append("No model call reached the recorder.")
            lines.extend(["", "Review: __________", ""])
    (directory / "review.md").write_text("\n".join(lines))


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        build(Path(arg).resolve())
