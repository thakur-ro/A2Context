"""Connect both public Collector pilot workflows to fresh sidecars."""

import argparse
from pathlib import Path
import sqlite3
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "transfer"))
from pilot import run

REVISION = "8e266ba207ab3ad93e3606578e3b56a5fbd47adf"
COMPONENTS = (
    "exporter/exporterhelper",
    "exporter/otlpexporter",
    "processor/batchprocessor",
    "processor/memorylimiterprocessor",
    "receiver/otlpreceiver",
)


def prepare(checkout, workflow, directory):
    def git(*args):
        return subprocess.check_output(["git", "-C", checkout, *args], text=True)

    if workflow == "documents":
        for component in COMPONENTS:
            path = component + "/README.md"
            dest = directory / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(git("show", f"{REVISION}:{path}"))
        return
    commits = git("log", "-100", "--first-parent", "--format=%H%x09%cs%x09%s", REVISION).splitlines()
    assert len(commits) == 100
    with sqlite3.connect(directory / "history.db") as db:
        db.execute("CREATE TABLE commits(sha TEXT PRIMARY KEY, date TEXT, subject TEXT)")
        db.execute("CREATE TABLE changes(sha TEXT, path TEXT)")
        for line in commits:
            sha, date, subject = line.split("\t", 2)
            db.execute("INSERT INTO commits VALUES(?,?,?)", (sha, date, subject))
            paths = git("diff", "--name-only", sha + "^1", sha).splitlines()
            db.executemany("INSERT INTO changes VALUES(?,?)", ((sha, p) for p in paths))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model-port", type=int, required=True)
    parser.add_argument(
        "--followup", action="store_true", help="Run the five fresh retrieval questions."
    )
    args = parser.parse_args()
    if args.followup:
        questions = [
            (line.split(". ", 1)[1],)
            for line in (HERE / "followup.md").read_text().splitlines()
            if line[:1].isdigit()
        ]
        run(
            args,
            HERE,
            (("citations", "documents"),),
            lambda name, directory: prepare(args.checkout, name, directory),
            questions,
        )
        sys.exit(0)
    run(
        args,
        HERE,
        (("sql", "history"), ("citations", "documents")),
        lambda name, directory: prepare(args.checkout, name, directory),
    )
