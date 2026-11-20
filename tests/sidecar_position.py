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
    record = {"question": "Where?", "ctxs": docs, "answers": ["A", "private-evaluation-alias"]}
    for rotation in [0, 5]:
        case = {"row": 0, "baseline": "A", "question_sha256": sha256(b"Where?").hexdigest(), "edits": []}
        for i, doc in enumerate(docs[rotation:] + docs[:rotation]):
            case["edits"].append({"position": i+1, "passage_sha256": sha256((doc["title"]+"\n"+doc["text"]).encode()).hexdigest(),
                                  "js_bits": 0.25 if doc is docs[0] else 0.0,
                                  "answer": "B" if doc is docs[0] and rotation == 0 else "A"})
        study = {"rotation": rotation, "finished_unix_ns": 1_800_000_000_000_000_000, "model": "fixture",
                 "revision": "1", "runtime": "unit", "tokenizer_sha256": "0" * 64}
        trace, case, outcomes = prepare(record, case, study, rankings=True, references=True)
        with TemporaryDirectory(dir=folder) as temporary:
            report = verified_report(binary, Path(temporary), trace, case, outcomes)
        send_traces(base, trace)
        missing = get(base, "ranking-examples", org="public-qa", result="missing")["rows"]
        assert any(r["trace"] == case["location"]["run_id"] and r["reference"] == {"status":"missing"} for r in missing)
        send(base, "/api/replay-reports", report, expected=201)
    return trace


def check(binary, folder):
    database = folder / "position.sqlite"
    with server(binary, database, "--capture-content") as base:
        captured = seed(base, binary, folder)
        ranking = [{"result":"both","calls":1,"same_edit":1},{"result":"neither","calls":1,"same_edit":0}]
        assert get(base, "ranking-results", org="public-qa")["rows"] == ranking
        loss, = get(base, "ranking-examples", org="public-qa", quality="ranked_lost")["rows"]
        assert loss["reference"] == {"status":"scored","original":True,"ranked":False,"first":False}
        assert not get(base, "ranking-examples", org="public-qa", quality="ranked_gained")["rows"]
        assert not get(base, "ranking-examples", org="another-workspace")["rows"]
        row, = get(base, "replay-variations", org="public-qa")["rows"]
        selection = {"factor": row["factor"], "group": row["group"], "pair": row["position_pair"],
                     "baseline_trials": 1, "edit_trials": 1, "org": "public-qa"}
        factor_scope = {"org": selection["org"], "factor": row["factor"]}
        factor, = get(base, "factors", **factor_scope)["rows"]
        assert factor["position_test"] is None
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
        top = get(base, "factors", org=selection["org"])["rows"][0]
        test = top["position_test"]
        assert top["id"] == selection["factor"] and test["result"]["present"] == {"changed": 2, "compared": 2}
        assert all(test[key] == selection[key] for key in ("pair", "group", "baseline_trials", "edit_trials"))
        for bg in plan["backgrounds"]:
            partial, = get(base, "factors", **factor_scope, trace=bg["location"]["run_id"])["rows"]
            assert partial["position_test"] is None
        assert get(base, "factors", org="another-workspace")["rows"] == []
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
    with server(binary, folder / "redacted.sqlite") as base:
        send_traces(base, captured)
        assert not get(base, "ranking-reference")["rows"]
        run_id = "otlp:" + captured["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"]
        assert "private-evaluation-alias" not in json.dumps(get(base, "trace", id=run_id))
    with sqlite3.connect(database) as db:
        db.execute("UPDATE observations SET detail=json_remove(detail,'$.selector_check','$.proposal_checks') WHERE kind='prefill'")
        db.execute("PRAGMA user_version=20")
    with server(binary, database, "--capture-content") as base:
        assert get(base, "ranking-results", org="public-qa")["rows"] == ranking
        loss, = get(base, "ranking-examples", org="public-qa", quality="ranked_lost")["rows"]
        assert loss["reference"] == {"status":"scored","original":True,"ranked":False,"first":False}
        assert not get(base, "ranking-examples", org="public-qa", quality="ranked_gained")["rows"]
        row, = get(base, "replay-variations", org="public-qa")["rows"]
        assert row["position_reports"] == 2 and row["latest_position_result"]["present"]["compared"] == 1
        assert row["latest_position_result"]["replaced"] == {"changed": 0, "compared": 2}
        latest, earlier = get(base, "position-results", **selection)["rows"]
        factor, = get(base, "factors", **factor_scope)["rows"]
        assert factor["position_test"]["result"] == latest
        assert earlier["present"] == {"changed": 2, "compared": 2}
        saved = get(base, "position-results", **selection, id=latest["id"])
        assert len(saved["trials"]) == 8
    # Even two correct trace IDs cannot identify a test if its compared sources have changed.
    with sqlite3.connect(database) as db:
        db.execute("UPDATE position_results SET pair=?", ("0"*64,))
    with server(binary, database, "--capture-content") as base:
        factor, = get(base, "factors", **factor_scope)["rows"]
        assert factor["position_test"] is None
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
