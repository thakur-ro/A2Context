use a2context::{conflict_query, langsmith, preflight, Run};
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
    missing.availability = a2context::Availability::Redacted;
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

#[test]
fn mines_verified_checks_across_scopes_and_opens_calls_without_output() {
    let raw: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns-langsmith.json")).unwrap();
    let mut first = langsmith::import(&raw).unwrap().remove(0);
    first.calls[0].decisions.clear();
    let mut second = first.clone();
    second.id = "another-call-same-thread".into();
    let mut third = first.clone();
    third.id = "another-thread".into();
    third.conversation_id = Some("another-thread".into());
    third.calls[0].started_at = Some("2026-01-16T00:00:00Z".into());
    let mut other = first.clone();
    other.id = "other-organization".into();
    other.organization_id = Some("other-organization".into());
    let mut modified = first.clone();
    modified.id = "unverified".into();
    modified.calls[0]
        .recorded_request
        .as_mut()
        .unwrap()
        .body
        .push(' ');
    let mut no_receipt = first.clone();
    no_receipt.id = "unchecked".into();
    no_receipt.calls[0].preflight_receipt = None;
    let source = [first, second, third, other, modified, no_receipt];
    let mut spec = json!({
        "version": 1, "phase": "preflight", "group_by": ["source_version", "day"],
        "scope": {"kind": "user", "organization_id": "demo-retail", "user_id": "demo-shopper"},
        "filters": {"rule": "return_window_days", "before": "2026-01-16T00:00:00Z"}, "limit": 1,
    });
    let page = query(spec.clone(), &source);
    assert_eq!(page["phase"], "preflight");
    assert_eq!(
        page["totals"],
        json!({
            "scoped_checks": 2, "comparable_checks": 2, "conflicting_checks": 2,
            "affected_conversations": 1, "conflicts_without_conversation": 0,
        })
    );
    assert_eq!(page["next_offset"], 1);
    assert_eq!(
        page["rows"][0]["key"],
        json!([["retail/archive-faq", "v2"], ["2026-01-15"]])
    );
    spec["group"] = page["rows"][0]["key"].clone();
    let evidence = query(spec.clone(), &source);
    assert_eq!(evidence["rows"][0]["call_id"], "returns-run");
    assert_eq!(
        evidence["rows"][0]["rule_scope"],
        json!(["return_window_days", "standard"])
    );
    assert!(evidence["rows"][0].get("decision_id").is_none());
    assert_eq!(evidence["next_offset"], 1);
    spec["offset"] = json!(1);
    let later = query(spec, &source);
    assert_eq!(later["rows"][0]["run_id"], "another-call-same-thread");
    assert!(later["next_offset"].is_null());

    let organizations = query(
        json!({"version": 1, "phase": "preflight", "group_by": ["organization"]}),
        &source,
    );
    assert_eq!(organizations["rows"][0]["conflicting_checks"], 3);
    assert_eq!(organizations["rows"][0]["affected_conversations"], 2);
    assert_eq!(organizations["rows"][1]["conflicting_checks"], 1);
    let conversations = query(
        json!({"version": 1, "phase": "preflight", "group_by": ["conversation"]}),
        &source,
    );
    assert_eq!(conversations["rows"].as_array().unwrap().len(), 3);
    assert_eq!(conversations["rows"][0]["conflicting_checks"], 2);
}

#[test]
fn counts_each_checked_rule_scope_and_does_not_mix_known_values_across_scopes() {
    let mut input: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns.json")).unwrap();
    let mut raw: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns-langsmith.json")).unwrap();
    let declarations = "a2context.context.semantics";
    input["scope"] = Value::Null;
    let sources = &mut input["attributes"][declarations]["request_sources"];
    let mut holiday = sources[0].clone();
    holiday["id"] = json!("holiday-policy");
    holiday["rule"]["scope"] = json!("holiday");
    sources.as_array_mut().unwrap().push(holiday);
    let metadata = &mut raw["extra"]["metadata"];
    metadata[declarations]["request_sources"] = sources.clone();
    let request = serde_json::from_value(input).unwrap();
    metadata["a2context.preflight"] =
        serde_json::to_value(preflight::check(request).unwrap().receipt).unwrap();
    let recorded = langsmith::import(&raw).unwrap();
    let result = query(
        json!({"version": 1, "phase": "preflight", "group_by": ["rule"]}),
        &recorded,
    );
    assert_eq!(result["totals"]["scoped_checks"], 2);
    assert_eq!(result["totals"]["comparable_checks"], 1);
    assert_eq!(result["totals"]["conflicting_checks"], 1);
    assert_eq!(
        result["rows"][1]["key"],
        json!([["return_window_days", "holiday"]])
    );
    assert_eq!(result["rows"][1]["comparable_checks"], 0);
}
