"""Check every recorded SQL proposal against the sidecar and LangChain's own argument parser."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import sqlite3
import sys

from langchain_community.agent_toolkits.sql.toolkit import SQLDatabaseToolkit
from langchain_community.utilities import SQLDatabase
from langchain_openai import ChatOpenAI
from pydantic import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts"))
from check_sidecar import send


def tool_validators():
    # get_tools constructs the shipped schemas. Neither the model nor any SQL tool runs.
    llm = ChatOpenAI(base_url="http://127.0.0.1:1/v1", api_key="unused", model="schema-only")
    toolkit = SQLDatabaseToolkit(llm=llm, db=SQLDatabase.from_uri("sqlite://"))
    return {t.name:t.get_input_schema() for t in toolkit.get_tools()}


def run(folder, endpoint):
    validators = tool_validators()
    with sqlite3.connect((folder / "sidecar.sqlite").resolve().as_uri() + "?mode=ro", uri=True) as db:
        for session, raw in db.execute("SELECT thread,run FROM traces ORDER BY thread"):
            trace = json.loads(raw)
            for call in sorted(trace["calls"], key=lambda c: c["started_at"]):
                body = json.loads(call["recorded_request"]["body"])
                for d in call["decisions"]:
                    if d["kind"] != "tool_call":
                        continue
                    args = d["outcome"]
                    if isinstance(args, str):
                        args = json.loads(args)
                    payload = {"request":body, "tool":d["label"], "arguments":args}
                    check = send(endpoint,"/api/sidecar/check-proposal",payload)[1]
                    try:
                        validators[d["label"]].model_validate(args)
                        errors = []
                    except ValidationError as exc:
                        errors = [{"location":list(e["loc"]), "type":e["type"]} for e in exc.errors()]
                    yield {"session":session, "trace":trace["id"], "call":call["id"], "decision":d["id"], "tool":d["label"],
                           "request_sha256":sha256(call["recorded_request"]["body"].encode()).hexdigest(),
                           "check":check, "framework_errors":errors}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    for row in run(args.folder, f"http://127.0.0.1:{args.port}"):
        print(json.dumps(row))
