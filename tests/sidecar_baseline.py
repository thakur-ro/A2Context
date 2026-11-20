"""Verify that unsuccessful baselines surface independently of any edited arm."""
from copy import deepcopy
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "examples"))
from check_sidecar import attrs, get, send, send_traces, server
from send_otlp_replay import prepare


def baseline_report(original, reason):
    report = deepcopy(original)
    report.pop("pair")
    report["variants"] = report["variants"][:1]
    report["id"] = "baseline-" + reason
    trial = {"status": "failed"}
    if reason != "no_completion":
        trial = {"status": "completed", "choice": {**report["recorded"], "outcome": "unavailable"}}
    if reason == "no_score":
        report["match_rule"] = {"kind": "text_capture", "pattern": "(\\d+)"}
    summary = {"attempted": 1, "completed": int(reason != "no_completion"), "matched": 0}
    if reason == "no_score":
        summary["unscored"] = 1
    report.update(baseline=[trial], baseline_summary=summary)
    report["variants"][0].update(trials=[trial], summary=summary, difference_pp=None)
    return report


def verify(binary):
    path = "/api/replay-reports"
    with TemporaryDirectory() as tmp:
        db = Path(tmp) / "replay.sqlite"
        with server(binary, db, "--capture-content") as base:
            for conversation in ("first", "second"):
                trace, original = prepare(conversation)
                trace["resourceSpans"][0]["resource"] = {"attributes": attrs({"service.name": "baseline-test"})}
                send_traces(base, trace)
                send(base, path, original, expected=201)
                for reason in ("no_completion", "no_match", "no_score"):
                    report = baseline_report(original, reason)
                    send(base, path, report, expected=201)
                    send(base, path, report, expected=200)
            rows = get(base, "findings", kind="baseline_check")["rows"]
            assert len(rows) == 3, [(r["kind"], r["occurrences"]) for r in rows]
            for card in rows:
                assert card["threads"] == card["occurrences"] == 2
                detail = card["evidence"]["detail"]
                assert detail["sources"] == [] and "effect_pp" not in detail
                assert detail["measurement"]["replay_id"] == "baseline-" + detail["reason"]
                assert detail["measurement"]["baseline"]["matched"] == 0
                assert card["action"]["next"]
                factor, = get(base, "factors", factor=detail["factor"])["rows"]
                assert factor["signal"] == "baseline_check"
            tampered = baseline_report(original, "no_match")
            tampered["id"] = "false-summary"
            tampered["baseline_summary"]["completed"] = 0
            send(base, path, tampered, expected=400)
            groups = {row["id"] for row in rows}
        with sqlite3.connect(db) as conn:
            conn.execute("DELETE FROM observations WHERE kind='baseline_check'")
            conn.execute("PRAGMA user_version=33")
        with server(binary, db, "--capture-content") as base:
            rows = get(base, "findings", kind="baseline_check")["rows"]
            assert {row["id"] for row in rows} == groups
            assert all(row["occurrences"] == 2 for row in rows)
    print("Verified failed, unscorable and mismatched replay baselines, exact report links, retry and migration.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
