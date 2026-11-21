"""Execute the frozen public workflow pilot, retaining raw results locally."""

import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess

from capture import capture, get, now
from check_sidecar import ROOT, server


HERE = Path(__file__).resolve().parent
FROZEN = "efc533203cfae310dc4fde57fdcf1ac60295dd98"


def questions(workflow):
    column = 2 if workflow == "tools" else 3
    table = [line.split("|") for line in (HERE / "protocol.md").read_text().splitlines()]
    selected = [row[column].strip() for row in table if len(row) == 5 and row[1].strip().isdigit()]
    assert len(selected) == 5
    return selected


def prepare(workflow, directory):
    if workflow == "papers":
        for name in ["methods", "validation", "foundational-algorithms", "research-review"]:
            text = subprocess.check_output(["git", "show", f"{FROZEN}:docs/{name}.md"], cwd=ROOT)
            (directory / f"{name}.txt").write_bytes(text)
        return
    records = subprocess.check_output(["git", "log", FROZEN, "--format=%x1e%H%x09%cs%x09%s", "--name-only"],
                                      text=True, cwd=ROOT)
    with sqlite3.connect(directory / "history.db") as db:
        db.execute("CREATE TABLE commits(sha TEXT PRIMARY KEY, date TEXT, subject TEXT)")
        db.execute("CREATE TABLE changes(sha TEXT, path TEXT)")
        for record in records.split("\x1e")[1:]:
            header, *paths = record.strip().splitlines()
            sha, date, subject = header.split("\t", 2)
            db.execute("INSERT INTO commits VALUES(?,?,?)", (sha, date, subject))
            db.executemany("INSERT INTO changes VALUES(?,?)", ((sha, p) for p in paths if p))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workflow", choices=["tools", "papers"])
    parser.add_argument("application", type=Path, help="llm executable for tools, PaperQA environment Python for papers")
    parser.add_argument("directory", type=Path, help="New directory for the database and review evidence")
    parser.add_argument("--model-port", type=int, required=True, help="Local MLX server with default_model loaded")
    parser.add_argument("--binary", type=Path, required=True, help="Sidecar built from the frozen revision")
    args = parser.parse_args()
    directory = args.directory.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    prepare(args.workflow, directory)
    env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG", "TMPDIR", "TIKTOKEN_CACHE_DIR") if k in os.environ}
    env.update(LLM_USER_PATH=str(directory), LITELLM_LOCAL_MODEL_COST_MAP="True", HF_HUB_OFFLINE="1",
               NO_PROXY="127.0.0.1,localhost", LITELLM_TELEMETRY="False")
    journal = directory / "events.jsonl"
    with server(args.binary.resolve(), directory / "sidecar.sqlite", "--capture-content") as base:
        assert get(base, "health")["coverage"]["traces"] == 0
        for i, question in enumerate(questions(args.workflow), 1):
            run = f"{args.workflow}-{i}"
            with capture(base, args.model_port, args.workflow, run, journal) as endpoint:
                if args.workflow == "papers":
                    command = [args.application, HERE / "paper_question.py", directory, endpoint, question]
                else:
                    (directory / "extra-openai-models.yaml").write_text(json.dumps([{
                        "model_id": "local-pilot", "model_name": "default_model", "api_base": endpoint,
                        "supports_tools": True, "can_stream": False}]))
                    command = [args.application, "-m", "local-pilot", "-T", 'SQLite("history.db")', question]
                with (directory / f"{run}.log").open("w") as out:
                    try:
                        outcome = subprocess.run(command, cwd=directory, env=env, stdout=out, stderr=out,
                                                 timeout=300).returncode
                    except subprocess.TimeoutExpired:
                        outcome = "timeout"
                with journal.open("a") as file:
                    print(json.dumps({"at": now(), "event": "application_exit", "run": run, "status": outcome}), file=file)
        for view in ["health", "findings", "tool-reference-score", "traces"]:
            (directory / f"{view}.json").write_text(json.dumps(get(base, view), indent=2))


if __name__ == "__main__":
    main()
