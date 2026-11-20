use nexusweave::{inspect, langsmith, otlp, preflight, turn_evidence, FindingKind, Run};
use serde_json::{json, Value};
use std::process::Command;

const EXAMPLE: &str = include_str!("../examples/preflight-returns.json");
const TRACE: &str = include_str!("../examples/preflight-returns-langsmith.json");
const SEMANTICS: &str = "nexusweave.context.semantics";

fn check(input: Value) -> Result<preflight::Report, String> {
    preflight::check(serde_json::from_value(input).unwrap())
}

fn fixture_trace() -> Value {
    serde_json::from_str(TRACE).unwrap()
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
    let output = Command::new(env!("CARGO_BIN_EXE_nexusweave"))
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
fn carries_the_same_source_identities_into_a_recorded_decision() {
    let input: Value = serde_json::from_str(EXAMPLE).unwrap();
    let before = check(input).unwrap();
    let trace = fixture_trace();
    assert_eq!(
        serde_json::to_value(&before.receipt).unwrap(),
        trace["extra"]["metadata"]["nexusweave.preflight"]
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
        "extra.metadata[\"nexusweave.preflight\"]"
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
            "nexusweave.request.body",
            json!(format!(
                "{} ",
                trace["extra"]["metadata"]["nexusweave.request.body"]
                    .as_str()
                    .unwrap()
            )),
        ),
        (SEMANTICS, {
            let mut semantics = trace["extra"]["metadata"][SEMANTICS].clone();
            semantics["request_sources"][0]["rule"]["value"] = json!(90);
            semantics
        }),
        ("nexusweave.preflight", json!("{invalid")),
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
            "nexusweave.request.body",
            metadata["nexusweave.request.body"]
                .as_str()
                .unwrap()
                .to_owned(),
        ),
        (SEMANTICS, metadata[SEMANTICS].to_string()),
        (
            "nexusweave.preflight",
            metadata["nexusweave.preflight"].to_string(),
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
    assert_eq!(view["preflight"]["anchor"]["field"], "nexusweave.preflight");
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
