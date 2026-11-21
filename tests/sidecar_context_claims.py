"""Exercise capture-grounded recommendations, comparison scoping and migration."""

from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_sidecar import attrs, export, get, send, send_traces, server
from sidecar_tool_reference_score import call


TOOL = {"type":"function","function":{"name":"catalog_list","description":"List available collections.",
        "parameters":{"type":"object","properties":{}}}}
QUESTION = {"role":"user","content":"Which records changed?"}
DECLINE = "The available tools cannot retrieve the records."
PROMPT = "Context\nref-abc123: price\nref-dab456: tax\n\n## Citation examples\nEx. (ref-ff9999)"
ANSWER = "Résumé — the fee was recorded (ref-ff9999)."


def span(number, request, answer, *, user="usr", env="dev", proposed=False):
    item = call(number, user=user)
    parts = [{"type":"text","content":answer}]
    if proposed:
        parts.append({"type":"tool_call","id":"abc","name":"catalog_list","arguments":{}})
    item["attributes"] += attrs({"a2context.request.body":json.dumps(request),
                                 "gen_ai.output.messages":json.dumps([{"role":"assistant","parts":parts}]),
                                 "deployment.environment.name":env})
    return item


def check(binary, browser=False):
    offer = {"messages":[QUESTION],"tools":[TOOL]}
    paired = dict(offer, messages=[QUESTION,{"role":"assistant","tool_calls":[
        {"id":"abc","type":"function","function":{"name":"catalog_list","arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"abc","content":"returned"}])
    cited = {"messages":[{"role":"user","content":PROMPT}]}
    lookup = deepcopy(TOOL)
    lookup["function"]["parameters"] = {"type":"object","properties":{"item":{"type":"string"}},"required":["item"]}
    declines = [span(1, offer, DECLINE), span(2, offer, DECLINE)]
    exclude = [span(3, dict(offer, tool_choice="none"), DECLINE), span(4, offer, DECLINE, proposed=True),
               span(5, dict(offer, tools=[lookup]), DECLINE), span(6, offer, "I will explain the records."),
               span(7, dict(paired, tools=[lookup]), DECLINE), span(8, cited, "The passage says (ref-abc123)."),
               span(9, cited, "The passage says (ref-ef7777)."),
               span(10, {"messages":[{"role":"user","content":PROMPT.replace("## Citation examples", "More text")}]}, ANSWER),
               span(11, {"messages":[{"role":"system","content":"ref-ff9999: another passage"}, *cited["messages"]]}, ANSWER)]
    instruction = "Use only the supplied documents."
    supplied = "https://docs.example/source"
    added = "https://docs.example/other"
    link_prompt = instruction + "\nFiscalité [source](" + supplied + ")"
    link_reply = f"Résumé\nSource 1: [source]({supplied})\nReference 2: [details]({added})"
    def source_request(text):
        return {"messages":[{"role":"user","content":text}]}
    links = span(17, source_request(link_prompt), link_reply)
    exclude.extend([
        span(18, source_request(link_prompt + " " + added), link_reply),
        span(19, source_request("Do not " + link_prompt), link_reply),
        span(20, dict(source_request(link_prompt), web_search_options={}), link_reply),
        span(21, {"messages":[{"role":"user","content":[{"type":"text","text":link_prompt}]}]}, link_reply),
        span(22, source_request(link_prompt + " " + added + "-longer"), link_reply),
    ])
    target = span(12, cited, ANSWER)
    contract = dict(offer, tools=[lookup])
    missing = span(16, contract, "", proposed=True)
    # No cross-user or changed-catalog example may be returned to the dev report.
    foreign = span(13, paired, "OK", user="another-user")
    changed = span(14, dict(paired,tools=[dict(TOOL,revision=2)]), "OK")
    match = span(15, paired, "OK", env="stage")
    flags = ("--capture-content","--metadata","deployment.environment.name","--retain-days","365")
    with TemporaryDirectory() as tmp:
        db = Path(tmp) / "claims.sqlite"
        with server(binary, db, *flags) as base:
            batch = export(*declines, *exclude, target, foreign, changed, missing, links)
            send_traces(base, batch)
            send_traces(base, batch)
            def card(kind, **scope):
                row, = get(base, "findings", kind=kind, user="usr", **scope)["rows"]
                return row
            link_card = card("reference_link")
            assert link_card["occurrences"] == 1 and link_card["evidence"]["call_id"] == links["spanId"]
            detail = link_card["evidence"]["detail"]
            seen, novel = detail["links"]
            def fragment(text, interval):
                a,b = interval
                return text.encode()[a:b].decode()
            assert fragment(link_prompt,detail["instruction_bytes"]) == detail["instruction"]
            assert fragment(link_prompt,seen["input"]["bytes"]) == seen["url"] == supplied
            assert novel["input"] is None and novel["url"] == added
            assert all(fragment(link_reply,r["output_bytes"]) == r["url"] for r in detail["links"])
            proposal = card("missing_tool_argument")
            witness = proposal["evidence"]["detail"]
            assert proposal["occurrences"] == 1 and witness["check"]["argument_pointer"] == "/item"
            def check_args(tool, args, catalog=None):
                return send(base, "/api/sidecar/check-proposal", {
                    "request": {"tools": [tool] if catalog is None else catalog},
                    "tool": "catalog_list", "arguments": args})[1]
            assert check_args(lookup, {}) == witness["check"]
            for value in ({"item":None}, '{"item":0}'):
                assert check_args(lookup, value)["status"] == "no_missing_field_observed"
            for value in (None, "incomplete", []):
                assert check_args(lookup, value)["status"] == "unassessed"
            assert check_args(lookup, {}, [lookup,lookup])["status"] == "unassessed"
            sibling = {"type":"function","name":"search","parameters":{"properties":{"tag":{"type":"string"}}}}
            result = check_args(lookup, {"tag":"x"}, [lookup,sibling])
            assert result["other_declarations"] == [{"field":"tag","tool":"search","pointer":"/tools/1/parameters/properties/tag"}]
            assert check_args(lookup, {"tag":"x"}, [lookup,sibling,sibling])["other_declarations"] == []
            nested = {"properties":{"batches":{"items":{"properties":{"opts":{"required":["a~/b"]}}}}}}
            for tool, path in [
                ({"type":"function","name":"catalog_list","parameters":nested},"parameters"),
                ({"name":"catalog_list","input_schema":nested},"input_schema"),
            ]:
                issue = check_args(tool, {"batches":[{},{"opts":{"other":123}}]})
                assert issue["schema_pointer"] == "/tools/0/" + path + "/properties/batches/items/properties/opts/required/0"
                assert issue["argument_pointer"] == "/batches/1/opts/a~0~1b" and issue["supplied_fields"] == ["other"]
            report = card("capability_claim")
            assert report["occurrences"] == report["threads"] == 2 and report["discovery_example"] is None
            evidence = report["evidence"]["detail"]
            assert evidence["pointer"] == "/tools/0/function" and evidence["reply"] == DECLINE
            send_traces(base, export(match))
            comparison = card("capability_claim")["discovery_example"]
            assert comparison["trace"] == "otlp:" + match["traceId"] and comparison["result"] == "/messages/2"
            assert card("capability_claim", meta_key="deployment.environment.name", meta_value="dev")["discovery_example"] is None
            citation = card("example_citation")
            assert citation["occurrences"] == 1 and citation["evidence"]["trace_id"] == "otlp:" + target["traceId"]
            detail = citation["evidence"]["detail"]
            assert detail["label_count"] == 2 and detail["label_examples"] == ["ref-abc123","ref-dab456"]
            for field, value in [("input_bytes",PROMPT),("output_bytes",ANSWER)]:
                start, end = detail[field]
                assert value.encode()[start:end].decode() == detail["key"] == "ref-ff9999"
        with sqlite3.connect(db) as con:
            con.execute("DELETE FROM observations WHERE kind IN ('example_citation','capability_claim','missing_tool_argument')")
            con.execute("DROP TABLE discovery_uses")
            con.execute("PRAGMA user_version=36")
        with server(binary, db, *flags) as base:
            assert card("missing_tool_argument") == proposal
            assert card("example_citation") == citation
            assert card("capability_claim")["discovery_example"] == comparison
            if browser:
                script = Path(__file__).with_name("context-claims-browser.mjs")
                subprocess.run(["node", script, base, "policy-workflow", "policy-workflow", "policy-workflow", "policy-workflow"], check=True)
        with sqlite3.connect(db) as con:
            con.execute("DELETE FROM observations WHERE kind='missing_tool_argument'")
            con.execute("PRAGMA user_version=37")
        with server(binary, db, *flags) as base:
            assert card("missing_tool_argument") == proposal
            assert card("capability_claim")["discovery_example"] == comparison
        with sqlite3.connect(db) as con:
            con.execute("DELETE FROM observations WHERE kind='reference_link'")
            con.execute("PRAGMA user_version=38")
        with server(binary, db, *flags) as base:
            assert card("reference_link") == link_card
        with server(binary, Path(tmp)/"redacted.sqlite") as base:
            send_traces(base, batch)
            assert not get(base,"findings")["rows"]
    print("Verified citation offsets, claim and catalog evidence, scope, retries, redaction and v36 migration.")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve(), "--browser" in sys.argv[2:])
