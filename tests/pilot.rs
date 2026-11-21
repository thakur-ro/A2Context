use a2context::{add_annotations, exgentic, inspect, Annotation, Availability, FindingKind, Run};
use serde_json::json;

fn support() -> Vec<Run> {
    serde_json::from_str(include_str!("../fixtures/support.json")).unwrap()
}

#[test]
fn classifies_the_expected_pairs_and_keeps_scope() {
    let runs = support();
    let report = inspect(&runs).unwrap();
    let conflict = report
        .findings
        .iter()
        .find(|finding| finding.run_id == "return-eligibility" && finding.call_id == "check")
        .unwrap();
    assert_eq!(conflict.kind, FindingKind::Conflict);
    assert_eq!(
        conflict.context_ids,
        ["returns-policy", "return-preference"]
    );
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.call_id == "exception"));
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.call_id == "duplicate" && finding.kind == FindingKind::Duplicate));
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.call_id == "redacted"
            && finding.kind == FindingKind::InsufficientEvidence));
}

#[test]
fn counts_turns_and_scopes_without_counting_reimports() {
    let mut runs = support();
    runs.push(runs[0].clone());
    let report = inspect(&runs).unwrap();
    assert_eq!(report.runs.len(), 5);
    let user_pattern = report
        .patterns
        .iter()
        .find(|pattern| {
            pattern.kind == FindingKind::Conflict
                && pattern.organization_id.as_deref() == Some("demo-retail")
                && pattern.user_id.as_deref() == Some("user-042")
        })
        .unwrap();
    assert_eq!(user_pattern.occurrences.len(), 3);
    assert_eq!(user_pattern.affected_turns, Some(2));
    assert_eq!(user_pattern.affected_conversations, Some(2));
    let organization_pattern = report
        .patterns
        .iter()
        .find(|pattern| {
            pattern.kind == FindingKind::Conflict
                && pattern.organization_id.as_deref() == Some("demo-retail")
                && pattern.user_id.is_none()
        })
        .unwrap();
    assert_eq!(organization_pattern.affected_conversations, Some(3));
}

#[test]
fn profiles_named_sources_even_without_a_decision_link() {
    let mut runs = support();
    let call = &mut runs[0].calls[0];
    let mut repeated = call.relationships[0].clone();
    repeated.relation = "candidate policy".into();
    call.relationships.push(repeated);
    runs.push(runs[0].clone());

    let report = inspect(&runs).unwrap();
    let policy = report
        .source_profiles
        .iter()
        .find(|profile| profile.source_id == "returns-policy")
        .unwrap();
    assert_eq!(policy.occurrences.len(), 8);
    let first = policy
        .occurrences
        .iter()
        .find(|item| item.run_id == "return-eligibility" && item.call_id == "check")
        .unwrap();
    assert_eq!(first.linked_decision_ids, ["eligibility"]);
    assert_eq!(first.text_sha256.as_ref().unwrap().len(), 64);
    assert!(policy
        .occurrences
        .iter()
        .any(|item| item.call_id == "exception" && item.linked_decision_ids.is_empty()));

    let preference = report
        .source_profiles
        .iter()
        .find(|profile| profile.source_id == "return-preference")
        .unwrap();
    let redacted = preference
        .occurrences
        .iter()
        .find(|item| item.source_version == Some("v5"))
        .unwrap();
    assert_eq!(redacted.availability, Availability::Redacted);
    assert_eq!(redacted.text_sha256, None);
    assert_eq!(redacted.conversation_id, Some("holiday-order"));
    assert!(redacted.linked_decision_ids.is_empty());
}

#[test]
fn imports_model_call_and_later_tool_result_with_anchors() {
    let input = json!([{"role":"system","parts":[{"type":"text","content":"Inspect the order."}]}]);
    let output = json!([{"role":"assistant","parts":[
        {"type":"tool_call","name":"lookup","id":"tool-5","arguments":{"order":"A"}}
    ]}]);
    let later = json!([{"role":"tool","parts":[
        {"type":"tool_call_response","id":"tool-5","response":{"eligible":true}}
    ]}]);
    let row = json!({
        "session_id":"example", "benchmark":"tau2_retail", "spans":[
            {"span_id":"second", "trace_id":"trace", "start_time":"2026-01-01T00:00:02Z",
                "attributes":{"gen_ai.input.messages":later.to_string()}},
            {"span_id":"first", "trace_id":"trace", "start_time":"2026-01-01T00:00:01Z",
                "attributes":{"gen_ai.input.messages":input, "gen_ai.output.messages":output.to_string()}}
        ]
    });
    let runs = [exgentic::import_row(&row).unwrap()];
    let report = inspect(&runs).unwrap();
    assert_eq!(runs[0].user_id, None);
    assert_eq!(runs[0].calls[0].context[0].evidence.message_index, Some(0));
    assert_eq!(
        runs[0].calls[0].decisions[0].evidence.span_id.as_deref(),
        Some("first")
    );
    let link = &report.tool_links[0];
    assert_eq!(link.call_id, "first");
    assert_eq!(link.result_call_id, "second");
    assert_eq!(link.result.evidence.part_index, Some(0));
    assert_eq!(link.result.outcome, Some(json!({"eligible":true})));
}

#[test]
fn rejects_conflicting_reimports_and_unsupported_versions() {
    let mut runs = support();
    let mut changed = runs[0].clone();
    changed.user_id = Some("different".into());
    runs.push(changed);
    assert!(inspect(&runs).unwrap_err().contains("different records"));
    runs.pop();
    runs[0].schema_version = 2;
    assert!(inspect(&runs)
        .unwrap_err()
        .contains("unsupported schema version"));
}

#[test]
fn annotations_are_atomic_and_require_real_excerpts() {
    let mut runs = support();
    let mut annotation: Annotation = serde_json::from_value(json!({
        "run_id":"return-eligibility", "call_id":"check",
        "relationship":{
            "decision_id":"eligibility", "context_id":"returns-policy",
            "relation":"sets cutoff", "basis":"declared",
            "excerpt":"30-day window"
        }
    }))
    .unwrap();
    let mut invalid = annotation.clone();
    invalid.relationship.excerpt = Some("90-day window".into());
    let before = runs.clone();
    assert!(add_annotations(&mut runs, vec![annotation.clone(), invalid]).is_err());
    assert_eq!(runs, before);
    add_annotations(&mut runs, vec![annotation.clone(), annotation.clone()]).unwrap();
    assert_eq!(runs[0].calls[0].relationships.len(), 2);
    annotation.call_id = "missing".into();
    assert!(add_annotations(&mut runs, vec![annotation]).is_err());
}
