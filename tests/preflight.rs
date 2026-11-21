use a2context::{inspect, langsmith, otlp, preflight, turn_evidence, FindingKind, Run};
use serde_json::{json, Value};
use std::process::Command;

const EXAMPLE: &str = include_str!("../examples/preflight-returns.json");
const TRACE: &str = include_str!("../examples/preflight-returns-langsmith.json");
const SEMANTICS: &str = "a2context.context.semantics";

fn check(input: Value) -> Result<preflight::Report, String> {
    preflight::check(serde_json::from_value(input).unwrap())
}

fn fixture_trace() -> Value {
    serde_json::from_str(TRACE).unwrap()
}

#[test]
fn measures_unions_of_shared_utf8_bytes_in_each_field() {
    let fragment = |id: usize, source: &str, pointer: &str, start: usize, end: usize| {
        json!({
            "id":format!("fragment-{id}"), "source_id":source,
            "request_fragment":{"pointer":pointer,"text_range":{"start_byte":start,"end_byte":end}}
        })
    };
    let mut request = json!({"version":1,"attributes":{
        "a2context.request.body":json!({"a/b":"éabcdef","other":"éabcdef"}).to_string(),
        "a2context.context.semantics":{"version":1,"request_sources":[
            fragment(0,"cache","/a~1b",0,5), fragment(1,"cache","/a~1b",2,6),
            fragment(2,"customer","/a~1b",3,7), fragment(3,"policy","/a~1b",4,5),
            fragment(4,"outside","/a~1b",7,8), fragment(5,"other","/other",0,8),
            fragment(6,"cache","/a~1b",2,6)
        ]}
    }});
    let overlap = check(request.clone()).unwrap().anchor_overlap.unwrap();
    assert_eq!(
        (
            overlap.named_bytes,
            overlap.shared_bytes,
            overlap.shared_fields
        ),
        (16, 3, 1)
    );
    assert_eq!(overlap.examples.len(), 1);
    assert_eq!(overlap.examples[0].pointer, "/a~1b");
    let [a, b] = &overlap.examples[0].sources;
    assert_eq!(
        (a.source_id.as_str(), b.source_id.as_str()),
        ("cache", "customer")
    );
    assert_eq!((a.text_range.start_byte, a.text_range.end_byte), (0, 5));
    assert_eq!((b.text_range.start_byte, b.text_range.end_byte), (3, 7));
    assert_eq!(overlap.examples[0].text_range.start_byte, 3);
    assert_eq!(overlap.examples[0].text_range.end_byte, 4);
    request["attributes"][SEMANTICS]["request_sources"] = json!([
        fragment(0, "a", "/other", 0, 2),
        fragment(1, "b", "/other", 2, 3)
    ]);
    assert_eq!(
        check(request.clone())
            .unwrap()
            .anchor_overlap
            .unwrap()
            .shared_bytes,
        0
    );
    request["attributes"][SEMANTICS]["request_sources"] = json!([]);
    assert!(check(request).unwrap().anchor_overlap.is_none());
}

#[test]
fn bounds_overlap_examples_without_truncating_the_measurement() {
    let fields = vec!["sample"; 12];
    let mut sources = Vec::new();
    for index in 0..fields.len() {
        for id in ["one", "two"] {
            sources.push(
                json!({"id":format!("{id}-{index}"), "source_id":id, "request_fragment":{
                    "pointer":format!("/inputs/{index}"),"text_range":{"start_byte":0,"end_byte":6}
                }}),
            );
        }
    }
    let request = json!({"version":1,"attributes":{"a2context.request.body":json!({"inputs":fields}).to_string(),
        "a2context.context.semantics":{"version":1,"request_sources":sources}}});
    let overlap = check(request).unwrap().anchor_overlap.unwrap();
    assert_eq!(
        (
            overlap.shared_fields,
            overlap.shared_bytes,
            overlap.named_bytes
        ),
        (12, 72, 72)
    );
    assert_eq!(overlap.examples.len(), 8);
}

fn at_turn(recorded: &[Run], include_excerpts: bool) -> Value {
    turn_evidence::query(
        &turn_evidence::Request {
            run_id: recorded[0].id.clone(),
            call_id: recorded[0].calls[0].id.clone(),
            decision_id: None,
            include_excerpts,
            offset: 0,
            limit: 1,
        },
        recorded,
        &[],
        true,
    )
    .unwrap()
}

#[test]
fn cli_reports_a_conflict_with_request_anchors_and_omits_raw_content() {
    let output = Command::new(env!("CARGO_BIN_EXE_a2context"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["preflight", "examples/preflight-returns.json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["captured_items"], 3);
    assert_eq!(report["scoped_rule_items"], 2);
    let finding = &report["findings"][0];
    assert_eq!(finding["kind"], "conflict");
    assert_eq!(finding["sources"][0]["value"], 60);
    assert_eq!(finding["sources"][1]["source_version"], "v12");
    assert_eq!(finding["sources"][1]["value"], 30);
    assert_eq!(
        finding["sources"][1]["request_fragment"]["pointer"],
        "/messages/0/content"
    );
    assert_eq!(
        finding["sources"][1]["request_fragment"]["text_range"]["end_byte"],
        24
    );
    assert!(!stdout.contains("Can I return this"));
    assert!(!stdout.contains("Company policy: 30 days."));
}

#[test]
fn checks_equal_missing_and_out_of_scope_values() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    input["attributes"][SEMANTICS]["request_sources"][1]["rule"]["value"] = json!(30);
    let equal = check(input.clone()).unwrap();
    assert_eq!(equal.findings[0].kind, FindingKind::Duplicate);

    input["attributes"][SEMANTICS]["request_sources"][1]["rule"]["value"] = Value::Null;
    let missing = check(input.clone()).unwrap();
    assert_eq!(missing.findings[0].kind, FindingKind::InsufficientEvidence);

    input["attributes"][SEMANTICS]["request_sources"][1]["rule"]["scope"] = json!("holiday");
    let scoped = check(input.clone()).unwrap();
    assert_eq!(scoped.scoped_rule_items, 1);
    assert!(scoped.findings.is_empty());
    input["scope"] = Value::Null;
    let all = check(input).unwrap();
    assert_eq!(all.scoped_rule_items, 2);
    assert!(all.findings.is_empty());
}

#[test]
fn explicit_scope_distinguishes_missing_capture_from_a_measured_count() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    for (scope, declarations, known_values) in [("standard", 2, 1), ("holiday", 0, 0)] {
        input["scope"]["scope"] = json!(scope);
        input["attributes"][SEMANTICS]["request_sources"][1]["rule"]["value"] = Value::Null;
        let coverage = check(input.clone()).unwrap().scope_coverage.unwrap();
        assert_eq!(
            (coverage.declarations, coverage.known_values),
            (declarations, known_values)
        );
        assert_eq!(coverage.scope.scope, scope);
    }
    input["scope"] = Value::Null;
    assert!(check(input).unwrap().scope_coverage.is_none());
}

#[test]
fn required_sources_match_complete_alternatives_at_validated_request_fragments() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    let required = "a2context.required_sources";
    let any = json!({"name":"Return eligibility","any_of":[
        ["retail/returns-policy", "order/receipt"], ["retail/archive-faq"]
    ]});
    input["attributes"][required] = any.clone();
    let initial = check(input.clone()).unwrap().source_coverage.unwrap();
    assert!(initial.matched);
    assert_eq!(initial.alternatives[0].unconfirmed, ["order/receipt"]);
    assert!(initial.alternatives[1].unconfirmed.is_empty());
    let mcp = a2context::pending::evidence(
        serde_json::from_value(json!({
            "input":{"request":input}
        }))
        .unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        mcp["preflight"]["source_coverage"],
        serde_json::to_value(initial).unwrap()
    );

    input["attributes"][SEMANTICS]["request_sources"]
        .as_array_mut()
        .unwrap()
        .pop();
    input["attributes"][required] = json!(any.to_string());
    let after = check(input.clone()).unwrap().source_coverage.unwrap();
    assert!(!after.matched);
    assert_eq!(after.alternatives[1].unconfirmed, ["retail/archive-faq"]);
    // The body still contains the FAQ's words. Only its source declaration disappeared.
    input["attributes"][SEMANTICS]["request_sources"][0]["request_fragment"]["text_range"]
        ["end_byte"] = json!(10000);
    assert!(check(input).is_err());
}

#[test]
fn a_source_name_in_unanchored_text_cannot_satisfy_a_capture_requirement() {
    let report = check(json!({"version":1,"attributes":{
        "gen_ai.input.messages":[{"role":"user","parts":[{"type":"text","content":"confirmed-consent"}]}],
        "a2context.required_sources":{"name":"Approval","any_of":[["confirmed-consent"]]}
    }})).unwrap();
    let coverage = report.source_coverage.unwrap();
    assert!(!coverage.matched);
    assert_eq!(coverage.alternatives[0].unconfirmed, ["confirmed-consent"]);
}

#[test]
fn pins_match_any_exact_fragment_with_the_named_id() {
    use sha2::{Digest, Sha256};
    let pin = format!("{:x}", Sha256::digest(" é ".as_bytes()));
    let mut input = json!({"version":1,"attributes":{
        "a2context.request.body":json!({"copies":["é"," é "]}).to_string(),
        "a2context.context.semantics":{"version":1,"request_sources":[
            {"id":"old","source_id":"policy","request_fragment":{"pointer":"/copies/0","text_range":{"start_byte":0,"end_byte":2}}},
            {"id":"new","source_id":"policy","request_fragment":{"pointer":"/copies/1","text_range":{"start_byte":0,"end_byte":4}}}
        ]},
        "a2context.required_sources":{"name":"Current policy","any_of":[["policy"],["fallback"]],
            "expected_text_sha256":{"policy":pin}}
    }});
    let check_input = |v: &Value| check(v.clone()).unwrap().source_coverage.unwrap();
    assert!(check_input(&input).matched);
    input["attributes"][SEMANTICS]["request_sources"]
        .as_array_mut()
        .unwrap()
        .pop();
    let coverage = check_input(&input);
    assert!(!coverage.matched);
    assert_eq!(
        coverage.text_mismatches.into_iter().collect::<Vec<_>>(),
        ["policy"]
    );
    input["attributes"][SEMANTICS]["request_sources"][0]["source_id"] = json!("fallback");
    let coverage = check_input(&input);
    assert!(coverage.matched);
    assert!(coverage.text_mismatches.is_empty());
    for pins in [
        json!({"other":pin}),
        json!({"policy":"x"}),
        json!({"policy":pin.to_uppercase()}),
    ] {
        input["attributes"]["a2context.required_sources"]["expected_text_sha256"] = pins;
        assert!(check(input.clone()).is_err());
    }
}

#[test]
fn empty_requirements_cannot_pass_vacuously() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    for groups in [json!([]), json!([[]]), json!([[" "]])] {
        input["attributes"]["a2context.required_sources"] =
            json!({"name":"Policy", "any_of":groups});
        assert!(check(input.clone()).is_err());
    }
    input["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("a2context.required_sources");
    assert!(check(input).unwrap().source_coverage.is_none());
}

#[test]
fn langsmith_recomputes_source_coverage_only_for_a_matching_version_two_receipt() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    input["attributes"]["a2context.required_sources"] = json!({
        "name":"Holiday return", "any_of":[["retail/returns-policy","holiday-calendar"]]
    });
    let receipt = serde_json::to_value(check(input).unwrap().receipt.unwrap()).unwrap();
    assert_eq!(receipt["version"], 2);
    let mut trace = fixture_trace();
    trace["extra"]["metadata"]["a2context.preflight"] = receipt;
    let runs = langsmith::import(&trace).unwrap();
    let result = at_turn(&runs, false);
    let coverage = &result["preflight"]["source_coverage"];
    assert_eq!(result["preflight"]["anchor_overlap"]["shared_bytes"], 0);
    assert_eq!(coverage["matched"], false);
    assert_eq!(
        coverage["alternatives"][0]["unconfirmed"],
        json!(["holiday-calendar"])
    );
    trace["extra"]["metadata"]["a2context.preflight"]["version"] = json!(1);
    let changed = at_turn(&langsmith::import(&trace).unwrap(), false);
    assert_eq!(changed["preflight"]["status"], "unverified");
    assert!(changed["preflight"].get("source_coverage").is_none());
    assert!(changed["preflight"].get("anchor_overlap").is_none());
}

#[test]
fn carries_the_same_source_identities_into_a_recorded_decision() {
    let input: Value = serde_json::from_str(EXAMPLE).unwrap();
    let before = check(input).unwrap();
    let trace = fixture_trace();
    assert_eq!(
        serde_json::to_value(&before.receipt).unwrap(),
        trace["extra"]["metadata"]["a2context.preflight"]
    );
    let recorded = langsmith::import(&trace).unwrap();
    let normalized = serde_json::to_string(&recorded).unwrap();
    let recorded: Vec<Run> = serde_json::from_str(&normalized).unwrap();
    let after = inspect(&recorded).unwrap();
    let [checked] = after.preflight_checks.as_slice() else {
        panic!("Expected one receipt")
    };
    let preflight::Verification::Matched { report } = &checked.verification else {
        panic!("Expected a matched check")
    };
    assert_eq!(report.findings[0].kind, FindingKind::Conflict);
    assert_eq!(
        checked.evidence.field,
        "extra.metadata[\"a2context.preflight\"]"
    );
    assert_eq!(after.findings.len(), 1);
    let finding = &after.findings[0];
    assert_eq!(finding.kind, before.findings[0].kind);
    for source in &before.findings[0].sources {
        assert!(finding.context_ids.contains(&source.context_id));
        let item = recorded[0].calls[0]
            .context
            .iter()
            .find(|item| item.id == source.context_id)
            .unwrap();
        assert_eq!(item.source_id, source.source_id);
        assert_eq!(item.source_version, source.source_version);
        assert_eq!(item.request_fragment, source.request_fragment);
    }
    let public = at_turn(&recorded, false);
    assert_eq!(public["preflight"]["finding_count"], 1);
    assert_eq!(public["rows"][0]["kind"], "preflight_finding");
    assert!(public["rows"][0]["inputs"][0]["rule_value"].is_null());
    let full = at_turn(&recorded, true);
    assert_eq!(full["rows"][0]["inputs"][0]["rule_value"], 60);
    assert_eq!(
        full["rows"][0]["inputs"][0]["request_fragment"]["text_range"]["start_byte"],
        25
    );
}

#[test]
fn rejects_changed_requests_and_rule_declarations_without_hiding_the_outcome() {
    let trace = fixture_trace();
    for (key, new_value) in [
        (
            "a2context.request.body",
            json!(format!(
                "{} ",
                trace["extra"]["metadata"]["a2context.request.body"]
                    .as_str()
                    .unwrap()
            )),
        ),
        (SEMANTICS, {
            let mut semantics = trace["extra"]["metadata"][SEMANTICS].clone();
            semantics["request_sources"][0]["rule"]["value"] = json!(90);
            semantics
        }),
        ("a2context.preflight", json!("{invalid")),
    ] {
        let mut changed = trace.clone();
        changed["extra"]["metadata"][key] = new_value;
        let recorded = langsmith::import(&changed).unwrap();
        let view = at_turn(&recorded, false);
        assert_eq!(view["preflight"]["status"], "unverified");
        assert_ne!(view["rows"][0]["kind"], "preflight_finding");
        assert_eq!(view["decision"]["outcome_captured"], true);
    }
}

#[test]
fn matches_an_otlp_receipt_with_the_same_request_across_adapters() {
    let mut trace = fixture_trace();
    let metadata = &mut trace["extra"]["metadata"];
    metadata[SEMANTICS]["decisions"][0]["anchor"] = json!({
        "field":"gen_ai.output.messages", "message_index":0, "part_index":0,
    });
    let attributes = [
        ("gen_ai.operation.name", "chat".to_owned()),
        (
            "gen_ai.output.messages",
            json!([{"role":"assistant","parts":[{"type":"text","content":"60 days"}]}]).to_string(),
        ),
        (
            "a2context.request.body",
            metadata["a2context.request.body"]
                .as_str()
                .unwrap()
                .to_owned(),
        ),
        (SEMANTICS, metadata[SEMANTICS].to_string()),
        (
            "a2context.preflight",
            metadata["a2context.preflight"].to_string(),
        ),
    ]
    .map(|(key, value)| json!({"key":key,"value":{"stringValue":value}}));
    let export = json!({"resourceSpans":[{"scopeSpans":[{"spans":[{
        "traceId":"11111111111111111111111111111111", "spanId":"2222222222222222",
        "attributes": attributes,
    }]}]}]});
    let recorded = otlp::import(&export).unwrap();
    let view = at_turn(&recorded, false);
    assert_eq!(view["preflight"]["status"], "matched");
    assert_eq!(view["preflight"]["anchor"]["field"], "a2context.preflight");
    assert_eq!(
        view["rows"][0]["inputs"][1]["source_id"],
        "retail/returns-policy"
    );
}

#[test]
fn rejects_output_attributes_and_mismatched_anchors() {
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    input["attributes"]["gen_ai.output.messages"] = json!([]);
    assert!(check(input.clone()).unwrap_err().contains("inputs only"));
    let attributes = input["attributes"].as_object_mut().unwrap();
    attributes.remove("gen_ai.output.messages");
    attributes[SEMANTICS]["context"] = json!([{ "source_id":"unmatched", "anchor": {
        "field":"gen_ai.input.messages", "message_index":0, "part_index":0
    }, "request_fragment": {"pointer":"/messages/0/content", "text_range": {"start_byte":0, "end_byte":24}} }]);
    assert!(check(input)
        .unwrap_err()
        .contains("does not match the captured body"));
}
