"""Execute the registered proposal-repair comparison without calling the proposed tools."""

import argparse
import json
from pathlib import Path
import re
import sqlite3

from pydantic import ValidationError
from check_proposals import send, tool_validators
from proposal_trials import trials


def evaluate(message, target, validator, history):
    calls = message.get("tool_calls") or []
    if len(calls) != 1:
        return {"result":"multiple_proposals" if calls else "no_proposal", "count":len(calls)}
    function = calls[0].get("function", {})
    if function.get("name") != target:
        return {"result":"other_tool", "tool":function.get("name")}
    try:
        args = function["arguments"]
        if isinstance(args, str):
            args = json.loads(args)
        validated = validator.model_validate(args)
    except (KeyError, ValueError, ValidationError):
        return {"result":"invalid_arguments"}
    query = validated.query
    result = {"result":"valid_proposal", "query":query, "compiled":False}
    if re.match(r"^\s*SELECT\b", query, re.I):
        try:
            history.execute("EXPLAIN QUERY PLAN " + query).fetchall()
            result["compiled"] = True
        except sqlite3.Error as exc:
            result["compile_error"] = str(exc)
    return result


def experiment(folder, sidecar, model):
    validators = tool_validators()
    with sqlite3.connect((folder/"history.db").resolve().as_uri()+"?mode=ro&immutable=1",uri=True) as history, \
         sqlite3.connect((folder/"sidecar.sqlite").resolve().as_uri()+"?mode=ro",uri=True) as captures:
        history.execute("PRAGMA query_only=ON")
        def authorize(op, table, name, schema, caller):
            allowed = (op == sqlite3.SQLITE_SELECT or (op == sqlite3.SQLITE_READ and table in {"commits","changes"})
                       or (op == sqlite3.SQLITE_FUNCTION and name.lower() in
                           {"count","min","max","sum","avg","date","strftime","like","lower","upper","substr","coalesce"}))
            return sqlite3.SQLITE_OK if allowed else sqlite3.SQLITE_DENY
        history.set_authorizer(authorize)
        for session, raw in captures.execute("SELECT thread,run FROM traces ORDER BY thread"):
            run = json.loads(raw)
            call = max(run["calls"], key=lambda c:c["started_at"])
            decision, = call["decisions"]
            if decision["kind"] != "tool_call":
                continue
            original = json.loads(call["recorded_request"]["body"])
            target, args, ident = decision["label"], decision["outcome"], decision["tool_call_id"]
            check = send(sidecar,"/api/sidecar/check-proposal",{"request":original,"tool":target,"arguments":args})[1]
            assert check["status"] == "missing_required" and target == "sql_db_query"
            proposal = {"role":"assistant","content":None,"tool_calls":[
                {"id":ident,"type":"function","function":{"name":target,"arguments":args}}]}
            generic = "This proposal has not run. The tool arguments are invalid."
            specific = generic + " The required argument " + json.dumps(check["field"]) + " is missing. Fields supplied: " + ", ".join(check["supplied_fields"]) + "."
            requests = {"original":original}
            for arm, feedback in [("generic",generic), ("specific",specific)]:
                reply = {"role":"tool", "tool_call_id":ident, "content":feedback+" Please submit a corrected proposal."}
                requests[arm] = dict(original, messages=[*original["messages"],proposal,reply])
            yield from trials(requests,session,call["id"],model,
                              lambda message: evaluate(message,target,validators[target],history))



if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder",type=Path)
    parser.add_argument("--model-port",type=int,required=True)
    parser.add_argument("--sidecar-port",type=int,required=True)
    args = parser.parse_args()
    for row in experiment(args.folder,f"http://127.0.0.1:{args.sidecar_port}",f"http://127.0.0.1:{args.model_port}"):
        print(json.dumps(row),flush=True)
