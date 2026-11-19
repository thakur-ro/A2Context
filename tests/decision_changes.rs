use nexusweave::{
    decision_changes::{compare_decisions, DecisionChangeStatus as Change},
    Decision, Evidence,
};
use serde_json::{json, Value};

fn tool(id: &str, label: &str, outcome: Value) -> Decision {
    Decision {
        id: id.into(),
        kind: "tool_call".into(),
        label: label.into(),
        outcome,
        evidence: Evidence {
            span_id: None,
            field: "output".into(),
            message_index: None,
            part_index: None,
        },
        tool_call_id: None,
        applies_to: None,
    }
}

#[test]
fn groups_recorded_tool_choices_and_preserves_each_anchor() {
    let before = [
        tool("a", "lookup", json!({})),
        tool("b", "lookup", json!({})),
    ];
    let later = [
        tool("c", "lookup", json!({})),
        tool("d", "escalate", Value::Null),
    ];
    let change = compare_decisions(&before, &later);
    assert_eq!(change.status, Change::MixChanged);
    let lookup = change
        .groups
        .iter()
        .find(|group| group.label == "lookup")
        .unwrap();
    assert_eq!(lookup.status, Change::MixChanged);
    assert_eq!(lookup.decision_ids, [vec!["a", "b"], vec!["c"]]);
    let escalation = change
        .groups
        .iter()
        .find(|group| group.label == "escalate")
        .unwrap();
    assert_eq!(escalation.decision_ids, [vec![], vec!["d"]]);
}

#[test]
fn compares_parallel_outcomes_by_multiplicity_and_exact_value() {
    let original = [
        tool("a", "lookup", json!({"version": 9223372036854775807u64})),
        tool("b", "lookup", json!({"version": 1})),
        tool("c", "lookup", json!({"version": 1})),
    ];
    let mut later = [
        original[1].clone(),
        original[0].clone(),
        original[2].clone(),
    ];
    assert_eq!(compare_decisions(&original, &later).status, Change::Same);
    later[1].outcome["version"] = json!(9223372036854775806u64);
    assert_eq!(
        compare_decisions(&original, &later).status,
        Change::OutcomeChanged
    );
    later[1] = original[0].clone();
    later[0].outcome["version"] = json!(9223372036854775807u64);
    assert_eq!(
        compare_decisions(&original, &later).status,
        Change::OutcomeChanged
    );
}

#[test]
fn distinguishes_incomplete_outcomes_from_definite_differences() {
    let before = [
        tool("a", "lookup", json!(30)),
        tool("b", "lookup", json!(30)),
        tool("c", "lookup", Value::Null),
    ];
    let mut after = [
        tool("d", "lookup", json!(60)),
        tool("e", "lookup", json!(30)),
        tool("f", "lookup", Value::Null),
    ];
    assert_eq!(
        compare_decisions(&before, &after).status,
        Change::Incomplete
    );
    after[1].outcome = json!(60);
    assert_eq!(
        compare_decisions(&before, &after).status,
        Change::OutcomeChanged
    );
    let unavailable = compare_decisions(&before, &[]);
    assert_eq!(unavailable.status, Change::Unavailable);
    assert_eq!(
        unavailable.groups[0].decision_ids,
        [vec!["a", "b", "c"], vec![]]
    );
}
