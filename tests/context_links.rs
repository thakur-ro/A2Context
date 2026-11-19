use nexusweave::{
    context_links::LinkKind,
    context_patterns::{ContextRelationship, ContextSource, LinkEvidence},
    inspect, Basis, Run,
};
use serde_json::json;

fn recorded() -> Vec<Run> {
    serde_json::from_value(json!([
        {
            "schema_version":1, "id":"first", "source":"workflow", "organization_id":"org-a",
            "user_id":"person", "conversation_id":"chat", "calls":[
                {"id":"lookup","turn_id":"turn","context":[],"tool_results":[],
                 "decisions":[{"id":"one", "kind":"tool_call","label":"lookup","tool_call_id":"record-1",
                    "outcome":{"email":"shopper@example.test"},"evidence":{"field":"output"}}]},
                {"id":"use","turn_id":"turn","decisions":[{"id":"two","kind":"tool_call","label":"details",
                    "outcome":{"customer_id":"record_123","status":"active","email":"shopper@example.test","zip":30251},
                    "evidence":{"field":"output"}}],
                 "tool_results":[{"tool_call_id":"record-1","outcome":"record_123","evidence":{"field":"result"}}],
                 "context":[
                    {"id":"definition","kind":"tool_definition","availability":"included",
                     "text":"{\"function\":{\"name\":\"details\"}}","evidence":{"field":"tool_spec"}},
                    {"id":"wrong-definition","kind":"tool_definition","availability":"included",
                     "text":"{\"name\":\"details_v2\"}","evidence":{"field":"tool_spec2"}},
                    {"id":"response","kind":"tool_call_response","availability":"included",
                     "text":"{\"id\":\"record-1\",\"result\":[{\"type\":\"text\",\"text\":\"record_123\"}]}",
                     "evidence":{"field":"result"}},
                    {"id":"structured","kind":"tool_result","availability":"included",
                     "text":"{\"zip\":30251,\"other_status\":\"active\"}","evidence":{"field":"result2"}},
                    {"id":"prefixed","kind":"message","role":"user","availability":"included",
                     "text":"The alternate address is other-shopper@example.test.","evidence":{"field":"text0"}},
                    {"id":"longer-domain","kind":"message","role":"user","availability":"included",
                     "text":"Try shopper@example.test.evil instead.","evidence":{"field":"text1"}},
                    {"id":"user","kind":"message","role":"user","availability":"included",
                     "text":"Please use shopper@example.test.","evidence":{"field":"text2"}},
                    {"id":"redacted","kind":"message","role":"user","availability":"redacted",
                     "evidence":{"field":"text3"}}
                 ]}
        ]}
    ])).unwrap()
}

#[test]
fn derives_links_from_captured_context_with_exact_boundaries() {
    let runs = recorded();
    let report = inspect(&runs).unwrap();
    let links = &report.context_links;
    assert_eq!(links.len(), 4);
    assert!(links
        .iter()
        .all(|link| link.call_id == "use" && link.decision_id == "two"));
    let definition = &links[0];
    assert_eq!(definition.kind, LinkKind::ToolDefinition);
    assert_eq!(definition.context_path.as_deref(), Some("/function/name"));
    let identifier = &links[1];
    assert_eq!(identifier.kind, LinkKind::ArgumentMatch);
    assert_eq!(identifier.argument_path.as_deref(), Some("/customer_id"));
    assert_eq!(identifier.context_path.as_deref(), Some("/0/text"));
    assert_eq!(identifier.source_tool_call_id, Some("record-1"));
    let origin = identifier.source_tool.unwrap();
    assert_eq!(
        (origin.call_id, origin.decision_id, origin.tool),
        ("lookup", "one", "lookup")
    );
    assert_eq!(links[2].argument_path.as_deref(), Some("/zip"));
    assert_eq!(links[3].context_id, "user");
    assert!(runs[0].calls[1].relationships.is_empty());
}

#[test]
fn links_structured_outcomes_to_the_same_field_and_value_with_decoded_paths() {
    let mut runs = recorded();
    let call = &mut runs[0].calls[1];
    let decision = &mut call.decisions[0];
    decision.kind = "decision_model".into();
    decision.outcome = json!({"answer":{"units/available":42,"status":"active","zip":30251}});
    call.context[2].text = Some(json!({
        "id":"record-1",
        "result":[{"type":"text","text":json!({"inventory":{"units/available":42,"other_status":"active"}}).to_string()}]
    }).to_string());
    call.context[3].text =
        Some(r#"{"zip":"30251","status":"inactive","units/available":"42"}"#.into());
    let mut second = runs[0].clone();
    second.id = "other-organization".into();
    second.organization_id = Some("org-b".into());
    runs.push(second);

    let view = inspect(&runs).unwrap();
    assert_eq!(view.context_links.len(), 2);
    let link = &view.context_links[0];
    assert_eq!(link.kind, LinkKind::OutcomeMatch);
    assert_eq!(link.context_id, "response");
    assert_eq!(
        link.outcome_path.as_deref(),
        Some("/answer/units~1available")
    );
    assert_eq!(
        link.context_path.as_deref(),
        Some("/0/text/inventory/units~1available")
    );
    assert_eq!(link.argument_path, None);
    assert_eq!(link.source_tool.unwrap().tool, "lookup");
    let group = view
        .context_patterns
        .iter()
        .find(|group| {
            matches!(
                group.key.relationship,
                ContextRelationship::OutcomeMatch { .. }
            )
        })
        .unwrap();
    assert_eq!(group.occurrences.len(), 2);
    assert_eq!(group.occurrences[0].organization_id, Some("org-a"));
    assert_eq!(group.occurrences[1].organization_id, Some("org-b"));
}

#[test]
fn links_a_whole_numeric_answer_only_when_one_typed_field_matches() {
    let mut runs = recorded();
    let call = &mut runs[0].calls[1];
    call.decisions[0].kind = "model_output".into();
    call.decisions[0].outcome = json!("42");
    call.context[3].text = Some(r#"{"quantity":42,"sku":"42","capacity":420}"#.into());
    let view = inspect(&runs).unwrap();
    assert_eq!(view.context_links.len(), 1);
    let link = &view.context_links[0];
    assert_eq!(link.kind, LinkKind::OutcomeMatch);
    assert_eq!(link.outcome_path.as_deref(), Some(""));
    assert_eq!(link.context_path.as_deref(), Some("/quantity"));

    runs[0].calls[1].context[3].text = Some(r#"{"quantity":42,"backorder":42}"#.into());
    assert!(inspect(&runs).unwrap().context_links.is_empty());
    runs[0].calls[1].context[3].text = Some(r#"{"quantity":"42"}"#.into());
    assert!(inspect(&runs).unwrap().context_links.is_empty());
}

#[test]
fn links_the_identifier_in_the_saved_inventory_answer_without_matching_prose_numbers() {
    let mut runs: Vec<Run> =
        serde_json::from_str(include_str!("../examples/local-mlx-tool/evidence.json")).unwrap();
    runs.retain(|run| run.id == "local-model-inventory-result");
    let view = inspect(&runs).unwrap();
    assert_eq!(view.context_links.len(), 1);
    let link = &view.context_links[0];
    assert_eq!(link.kind, LinkKind::OutcomeIdentifier);
    assert_eq!(link.outcome_path, None);
    assert_eq!(link.context_path.as_deref(), Some("/sku"));
    assert_eq!(link.excerpt, "MUG-42");

    for prose in [
        "There are 42 units.",
        "There are 42 units of MUG-420.",
        "There are 42 units of XL-MUG-42.",
    ] {
        runs[0].calls[0].decisions[0].outcome = json!(prose);
        assert!(inspect(&runs).unwrap().context_links.is_empty());
    }
}

#[test]
fn groups_tool_result_handoffs_across_distinct_values_and_tracks_the_original_scope() {
    let mut runs = recorded();
    let mut repeat = runs[0].clone();
    repeat.id = "second".into();
    repeat.organization_id = Some("org-b".into());
    let call = &mut repeat.calls[1];
    call.decisions[0].outcome["customer_id"] = json!("record_789");
    call.context[2].text = call.context[2]
        .text
        .as_ref()
        .map(|text| text.replace("record_123", "record_789"));
    call.tool_results[0].outcome = Some(json!("record_789"));
    runs.extend([repeat.clone(), repeat]);
    let inspection = inspect(&runs).unwrap();
    let result = inspection
        .context_patterns
        .iter()
        .find(|pattern| {
            matches!(
                pattern.key.source,
                ContextSource::ToolResult { tool: "lookup" }
            )
        })
        .unwrap();
    assert_eq!(result.key.decision_label, "details");
    assert_eq!(result.occurrences.len(), 2);
    let (first, second) = (&result.occurrences[0], &result.occurrences[1]);
    assert_eq!(first.conversation_id, second.conversation_id);
    assert_eq!(first.user_id, second.user_id);
    assert_eq!(first.organization_id, Some("org-a"));
    assert_eq!(second.organization_id, Some("org-b"));
    let values: Vec<_> = result
        .occurrences
        .iter()
        .map(|occurrence| {
            let LinkEvidence::ExactMatch { index } = occurrence.evidence else {
                panic!("expected exact match")
            };
            let matched = &inspection.context_links[index];
            assert_eq!(matched.context_id, occurrence.context_id);
            assert_eq!(matched.argument_path.as_deref(), Some("/customer_id"));
            matched.excerpt.as_str()
        })
        .collect();
    assert_eq!(values, ["record_123", "record_789"]);
}

#[test]
fn leaves_an_ambiguous_origin_unresolved_and_preserves_annotation_basis() {
    let mut runs = recorded();
    let mut duplicate = runs[0].calls[0].clone();
    duplicate.id = "different-lookup".into();
    runs[0].calls.insert(1, duplicate);
    let call = &mut runs[0].calls[2];
    call.context[2].source_id = Some("customer-source".into());
    call.context[2].source_version = Some("v1".into());
    call.relationships = serde_json::from_value(json!([
        {"decision_id":"two", "context_id":"response", "basis":"declared", "relation":"supplies customer ID"},
        {"decision_id":"two", "context_id":"response", "basis":"hypothesis", "relation":"supplies customer ID"}
    ])).unwrap();
    let inspection = inspect(&runs).unwrap();
    let matches: Vec<_> = inspection
        .context_links
        .iter()
        .filter(|link| link.context_id == "response")
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].source_tool_call_id, Some("record-1"));
    assert!(matches[0].source_tool.is_none());
    assert!(!inspection
        .context_patterns
        .iter()
        .any(|pattern| matches!(pattern.key.source, ContextSource::ToolResult { .. })));
    for (index, basis) in [Basis::Declared, Basis::Hypothesis].into_iter().enumerate() {
        let pattern = inspection
            .context_patterns
            .iter()
            .find(|pattern| {
                matches!(
                    pattern.key.relationship, ContextRelationship::Annotation {
                        basis: annotation, relation: "supplies customer ID",
                    } if annotation == basis
                )
            })
            .unwrap();
        assert!(matches!(
            pattern.key.source,
            ContextSource::Named {
                id: "customer-source",
                version: Some("v1"),
                ..
            }
        ));
        assert_eq!(pattern.occurrences.len(), 1);
        assert!(
            matches!(pattern.occurrences[0].evidence, LinkEvidence::Annotation { index: pointer } if pointer == index)
        );
    }
}

#[test]
fn groups_tool_calls_by_org_and_user_with_distinct_turn_counts() {
    let mut runs = recorded();
    let mut repeat = runs[0].clone();
    repeat.id = "followup".into();
    let mut other_org = repeat.clone();
    other_org.id = "different-org".into();
    other_org.organization_id = Some("org-b".into());
    runs.extend([repeat, other_org]);
    let report = inspect(&runs).unwrap();
    let pattern = report
        .tool_patterns
        .iter()
        .find(|pattern| {
            pattern.tool == "details"
                && pattern.organization_id == Some("org-a")
                && pattern.user_id == Some("person")
        })
        .unwrap();
    assert_eq!(pattern.run_count, 2);
    assert_eq!(pattern.conversation_count, Some(1));
    assert_eq!(pattern.turn_count, Some(1));
    assert_eq!(pattern.linked_decisions, 2);
    assert_eq!(pattern.argument_matches, 2);
    assert_eq!(pattern.occurrences.len(), 2);
    let all = report
        .tool_patterns
        .iter()
        .find(|pattern| {
            pattern.tool == "details"
                && pattern.organization_id.is_none()
                && pattern.user_id.is_none()
        })
        .unwrap();
    assert_eq!(all.run_count, 3);
    assert_eq!(all.conversation_count, Some(2));
}
