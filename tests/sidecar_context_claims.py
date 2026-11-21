"""Exercise capture-grounded recommendations, comparison scoping and migration."""

from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_sidecar import attrs, export, get, send_traces, server
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
    target = span(12, cited, ANSWER)
    # No cross-user or changed-catalog example may be returned to the dev report.
    foreign = span(13, paired, "OK", user="another-user")
    changed = span(14, dict(paired,tools=[dict(TOOL,revision=2)]), "OK")
    match = span(15, paired, "OK", env="stage")
    flags = ("--capture-content","--metadata","deployment.environment.name","--retain-days","365")
    with TemporaryDirectory() as tmp:
        db = Path(tmp) / "claims.sqlite"
        with server(binary, db, *flags) as base:
            batch = export(*declines, *exclude, target, foreign, changed)
            send_traces(base, batch)
            send_traces(base, batch)
            def card(kind, **scope):
                row, = get(base, "findings", kind=kind, user="usr", **scope)["rows"]
                return row
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
            con.execute("DELETE FROM observations WHERE kind IN ('example_citation','capability_claim')")
            con.execute("DROP TABLE discovery_uses")
            con.execute("PRAGMA user_version=36")
        with server(binary, db, *flags) as base:
            assert card("example_citation") == citation
            assert card("capability_claim")["discovery_example"] == comparison
            if browser:
                script = Path(__file__).with_name("context-claims-browser.mjs")
                subprocess.run(["node", script, base, "policy-workflow", "policy-workflow"], check=True)
        with server(binary, Path(tmp)/"redacted.sqlite") as base:
            send_traces(base, batch)
            assert not get(base,"findings")["rows"]
    print("Verified citation offsets, claim and catalog evidence, scope, retries, redaction and v36 migration.")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve(), "--browser" in sys.argv[2:])
