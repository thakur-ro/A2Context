"""Run the frozen transfer protocol and retain the raw model and application records locally."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from capture import capture, get, now
from check_sidecar import server
from run import prepare


def run(args, study=HERE, workflows=(("sql", "tools"), ("citations", "papers")), prepare_corpus=prepare, questions=None):
    root = args.directory.resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = {k: os.environ[k] for k in ("HOME", "PATH", "LANG", "TMPDIR") if k in os.environ}
    env.update(HF_HUB_OFFLINE="1", NO_PROXY="127.0.0.1,localhost", TOKENIZERS_PARALLELISM="false",
               NUMBA_CACHE_DIR=str(root / "cache"))
    if questions is None:
        table = [line.split("|") for line in (study / "protocol.md").read_text().splitlines()]
        questions = [row[2:4] for row in table if len(row) == 5 and row[1].strip().isdigit()]
    assert len(questions) == 5
    for column, (workflow, corpus) in enumerate(workflows):
        directory = root / workflow
        directory.mkdir()
        prepare_corpus(corpus, directory)
        journal = directory / "events.jsonl"
        with server(args.binary.resolve(), directory / "sidecar.sqlite", "--capture-content") as base:
            assert get(base, "health")["coverage"]["traces"] == 0
            for n, row in enumerate(questions, 1):
                run_id = f"{workflow}-{n}"
                with capture(base, args.model_port, workflow, run_id, journal) as endpoint:
                    command = [args.python.absolute(), study / "workflow.py", workflow, directory, endpoint, row[column].strip()]
                    with (directory / (run_id + ".log")).open("w") as log:
                        try:
                            status = subprocess.run(command, cwd=directory, env=env, stdout=log, stderr=log,
                                                    timeout=300).returncode
                        except subprocess.TimeoutExpired:
                            status = "timeout"
                    with journal.open("a") as log:
                        print(json.dumps({"event":"application_exit","at":now(),"run":run_id,"status":status}),file=log)
                    print(run_id, status, flush=True)
            for view in ("health", "findings", "traces", "tool-reference-score"):
                (directory / (view + ".json")).write_text(json.dumps(get(base, view), indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model-port", type=int, required=True)
    run(parser.parse_args())
