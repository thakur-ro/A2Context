use nexusweave::{inspect, langsmith, preflight, FindingKind};
use serde_json::{json, Value};
use std::process::Command;

const EXAMPLE: &str = include_str!("../examples/preflight-returns.json");
const SEMANTICS: &str = "nexusweave.context.semantics";

fn check(input: Value) -> Result<preflight::Report, String> {
    preflight::check(serde_json::from_value(input).unwrap())
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
fn carries_the_same_source_identities_into_a_recorded_decision() {
    let input: Value = serde_json::from_str(EXAMPLE).unwrap();
    let before = check(input.clone()).unwrap();
    let mut metadata = json!({"nexusweave.span.id":"returns-run"});
    metadata["nexusweave.request.body"] = input["attributes"]["nexusweave.request.body"].clone();
    metadata[SEMANTICS] = input["attributes"][SEMANTICS].clone();
    metadata[SEMANTICS]["decisions"] = json!([{
        "anchor": {"field": "outputs.output"},
        "applies_to": input["scope"]
    }]);
    let recorded = langsmith::import(&json!({
        "id":"returns-run", "run_type":"llm",
        "outputs":{"output":"You have 60 days to return it."},
        "extra":{"metadata":metadata}
    }))
    .unwrap();
    let after = inspect(&recorded).unwrap();
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
