"""Exercise position-plan download, execution, scoped result storage, and expiry."""
from copy import deepcopy
from hashlib import sha256
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "scripts"), str(ROOT / "examples/public-qa")]
from check_sidecar import get, send, send_traces, server
from view_passage_edits import prepare, verified_report


def seed(base, binary, folder):
    docs = [{"title": str(i), "text": f"café {i}"} for i in range(10)]
    record = {"question": "Where?", "ctxs": docs}
    for rotation in [0, 5]:
        case = {"row": 0, "baseline": "A", "question_sha256": sha256(b"Where?").hexdigest(), "edits": []}
        for i, doc in enumerate(docs[rotation:] + docs[:rotation]):
            case["edits"].append({"position": i+1, "passage_sha256": sha256((doc["title"]+"\n"+doc["text"]).encode()).hexdigest(),
                                  "answer": "B" if doc is docs[0] and rotation == 0 else "A"})
        study = {"rotation": rotation, "finished_unix_ns": 1_800_000_000_000_000_000, "model": "fixture",
                 "revision": "1", "runtime": "unit", "tokenizer_sha256": "synthetic"}
        trace, case, outcomes = prepare(record, case, study)
        with TemporaryDirectory(dir=folder) as temporary:
            report = verified_report(binary, Path(temporary), trace, case, outcomes)
        send_traces(base, trace)
        send(base, "/api/replay-reports", report, expected=201)


def check(binary, folder):
    database = folder / "position.sqlite"
    with server(binary, database, "--capture-content") as base:
        seed(base, binary, folder)
        row, = get(base, "replay-variations", org="public-qa")["rows"]
        selection = {"factor": row["factor"], "group": row["group"], "pair": row["position_pair"],
                     "baseline_trials": 1, "edit_trials": 1, "org": "public-qa"}
        plan = get(base, "position-plan", **selection)
        plan_path, result_path = folder/"plan", folder/"result"
        plan_path.write_text(json.dumps(plan))
        command = [binary, "run-position-plan", plan_path, "fixture", result_path, "--", sys.executable, __file__, "--proposal"]
        subprocess.run(command, check=True)
        result = json.loads(result_path.read_text())
        assert len(result["trials"]) == 8
        # Reordered uploads with identical observations refer to the same receipt.
        payload = {"selection": selection, "result": result}
        send(base, "/api/position-results", payload, expected=201)
        result["trials"].reverse()
        send(base, "/api/position-results", payload)
        for field in ("pair", "user", "baseline_trials"):
            invalid = {**selection, field: "9"}
            send(base, "/api/position-results", {**payload, "selection": invalid}, expected=400)
        corrupt = deepcopy(result)
        corrupt["trials"][0]["request_sha256"] = "0"*64
        send(base, "/api/position-results", {**payload, "result": corrupt}, expected=400)
        incomplete = deepcopy(result)
        intact = next(r for r in incomplete["trials"] if not r["source_replaced"])
        intact["trial"] = {"status": "failed"}
        send(base, "/api/position-results", {**payload, "result": incomplete}, expected=201)
        # Invalid plans must fail before starting a runner or creating output.
        plan["requests"][next(iter(plan["requests"]))] += " "
        plan_path.write_text(json.dumps(plan))
        command[4] = folder/"invalid-result"
        assert subprocess.run(command, capture_output=True).returncode != 0 and not command[4].exists()
    with server(binary, database, "--capture-content") as base:
        row, = get(base, "replay-variations", org="public-qa")["rows"]
        assert row["position_reports"] == 2 and row["latest_position_result"]["present"]["compared"] == 1
        assert row["latest_position_result"]["replaced"] == {"changed": 0, "compared": 2}
        latest, earlier = get(base, "position-results", **selection)["rows"]
        assert earlier["present"] == {"changed": 2, "compared": 2}
        saved = get(base, "position-results", **selection, id=latest["id"])
        assert len(saved["trials"]) == 8
    with sqlite3.connect(database) as db:
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("DELETE FROM traces WHERE id=?", (plan["backgrounds"][1]["location"]["run_id"],))
        assert db.execute("SELECT count(*) FROM position_results").fetchone() == (0,)


if __name__ == "__main__":
    if sys.argv[1:] == ["--proposal"]:
        text = json.load(sys.stdin)["messages"][1]["content"]
        answer = "B" if text.startswith("[1] 0\n") else "A"
        print(json.dumps({"kind": "model_output", "label": "Answer", "outcome": answer}))
    else:
        with TemporaryDirectory() as tmp:
            check(Path(sys.argv[1]).resolve(), Path(tmp))
        print("Position experiment passed, including retries, incomplete trials, restart and retention.")
