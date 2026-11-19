use nexusweave::{conflict_query, Run};
use serde_json::{json, Value};

fn runs(source: &str) -> Vec<Run> {
    serde_json::from_str(source).unwrap()
}

fn query(spec: Value, source: &[Run]) -> Value {
    let spec = serde_json::from_value(spec).unwrap();
    serde_json::to_value(conflict_query::query(&spec, source).unwrap()).unwrap()
}

#[test]
fn keeps_identical_user_ids_separate_and_tracks_unknown_conversations() {
    let mut source = runs(include_str!("../fixtures/support.json"));
    let mut unassigned = source[0].clone();
    unassigned.id = "unknown-conversation".into();
    unassigned.conversation_id = None;
    source.push(unassigned);

    let scoped = query(
        json!({
            "version": 1,
            "group_by": ["organization"],
            "scope": {"kind": "user", "organization_id": "demo-retail", "user_id": "user-042"}
        }),
        &source,
    );
    assert_eq!(scoped["rows"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["rows"][0]["key"], json!([["demo-retail"]]));
    assert_eq!(
        scoped["totals"],
        json!({
            "scoped_decisions": 8, "comparable_decisions": 6, "conflicting_decisions": 5,
            "affected_conversations": 2, "conflicts_without_conversation": 2,
        })
    );

    let other = query(
        json!({
            "version": 1,
            "group_by": ["user"],
            "scope": {"kind": "organization", "organization_id": "second-retailer"}
        }),
        &source,
    );
    assert_eq!(
        other["rows"][0]["key"],
        json!([["second-retailer", "user-042"]])
    );
    assert_eq!(other["totals"]["conflicting_decisions"], 1);
}

#[test]
fn de_duplicates_pairs_and_returns_only_contributing_decisions_for_a_source() {
    let mut source = runs(include_str!("../fixtures/conversation.json"));
    let context = &mut source[0].calls[2].context;
    let mut repeated = context[0].clone();
    repeated.id = "policy-copy".into();
    context.push(repeated);
    let mut missing = context[0].clone();
    missing.id = "policy-unavailable".into();
    missing.source_id = Some("Archived policy".into());
    missing.text = None;
    missing.availability = nexusweave::Availability::Redacted;
    missing.rule.as_mut().unwrap().value = None;
    context.push(missing);
    let spec = json!({
        "version": 1, "group_by": ["source_version", "model"],
        "filters": {"rule": "return_window_days"},
    });
    let result = query(spec.clone(), &source);
    assert_eq!(result["totals"]["scoped_decisions"], 2);
    assert_eq!(result["totals"]["conflicting_decisions"], 1);
    let rows = result["rows"].as_array().unwrap();
    let policy = rows
        .iter()
        .find(|row| row["key"][0][0] == "Returns policy")
        .unwrap();
    assert_eq!(policy["scoped_decisions"], 2);
    assert_eq!(policy["comparable_decisions"], 1);
    assert_eq!(policy["conflicting_decisions"], 1);
    let archived = rows
        .iter()
        .find(|row| row["key"][0][0] == "Archived policy")
        .unwrap();
    assert_eq!(archived["comparable_decisions"], 0);
    assert_eq!(archived["conflicting_decisions"], 0);

    let mut details = spec;
    details["group"] = policy["key"].clone();
    let result = query(details, &source);
    assert_eq!(result["kind"], "evidence");
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["rows"][0]["call_id"], "jacket-answer");
    assert_eq!(result["rows"][0]["turn_id"], "jacket-eligibility");
}

#[test]
fn supports_utc_day_grouping_windows_and_pages() {
    let mut source = runs(include_str!("../fixtures/conversation.json"));
    source[0].calls[0].started_at = Some("2026-11-17T23:50:00-01:00".into());
    let mut spec = json!({
        "version": 1, "group_by": ["day", "source_version"], "limit": 1,
        "filters": {"after": "2026-11-18T00:00:00Z", "before": "2026-11-19T00:00:00Z"}
    });
    let first = query(spec.clone(), &source);
    assert_eq!(first["next_offset"], 1);
    assert_eq!(first["rows"][0]["key"][0], json!(["2026-11-18"]));
    assert_eq!(first["totals"]["scoped_decisions"], 2);
    spec["offset"] = json!(1);
    let second = query(spec, &source);
    assert_ne!(first["rows"][0]["key"], second["rows"][0]["key"]);
    assert!(second["next_offset"].is_null());
}

#[test]
fn rejects_ambiguous_or_unbounded_specs() {
    for spec in [
        json!({"version": 2, "group_by": ["rule"]}),
        json!({"version": 1, "group_by": ["rule", "rule"]}),
        json!({"version": 1, "group_by": ["model"], "limit": 101}),
        json!({"version": 1, "group_by": ["rule"], "group": [["bad"]]}),
        json!({"version": 1, "group_by": ["day"], "filters": {"after": "yesterday"}}),
    ] {
        let spec = serde_json::from_value(spec).unwrap();
        assert!(conflict_query::query(&spec, &[]).is_err());
    }
    assert!(serde_json::from_value::<conflict_query::ViewSpec>(
        json!({"version": 1, "group_by": ["undeclared_workflow"]})
    )
    .is_err());
}
