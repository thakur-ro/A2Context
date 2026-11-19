use nexusweave::{langsmith, replay::candidates, Run};
use serde_json::{json, Value};

const TRACE: &str = include_str!("../examples/runtime-comparison/pending-routing/trace.json");

fn sample() -> Run {
    let trace = serde_json::from_str(TRACE).unwrap();
    langsmith::import(&trace).unwrap().remove(0)
}

fn query(spec: Value, runs: &[Run]) -> Value {
    candidates::patterns::query(&serde_json::from_value(spec).unwrap(), runs).unwrap()
}

#[test]
fn compares_organizations_and_counts_distinct_threads_even_with_repeated_probes() {
    let mut first = sample();
    let reports = first.calls[0]
        .candidate_probes
        .as_mut()
        .unwrap()
        .reports
        .as_mut()
        .unwrap();
    let mut repeated = reports[0].clone();
    repeated.id = "second-probe-same-call".into();
    reports.push(repeated);
    let mut second = sample();
    second.id = "second-conversation".into();
    second.conversation_id = Some("second-thread".into());
    second.calls[0].started_at = Some("2026-01-16T00:00:00Z".into());
    second.calls[0].decisions.clear();
    let mut comparison = sample();
    comparison.id = "another-org".into();
    comparison.organization_id = Some("other-org".into());
    let mut stale = sample();
    stale.id = "stale".into();
    stale.calls[0]
        .recorded_request
        .as_mut()
        .unwrap()
        .body
        .push(' ');
    let runs = [second, stale, comparison, first];

    let mut spec = json!({
        "version": 1, "source_id": "request-type", "min_conversations": 2, "limit": 1,
        "selection": { "scope": {"kind":"user", "organization_id":"demo-retail", "user_id":"demo-shopper"}},
        "compare": { "scope": {"kind":"organization", "organization_id":"other-org"}},
    });
    let page = query(spec.clone(), &runs);
    let group = &page["groups"][0];
    assert_eq!(page["total_groups"], 1);
    assert_eq!(group["primary"]["measurements"], 3);
    assert_eq!(group["primary"]["calls"], 2);
    assert_eq!(group["primary"]["conversations"], 2);
    assert_eq!(group["primary"]["reversals"], 3);
    assert_eq!(group["comparison"]["measurements"], 1);
    assert_eq!(group["comparison"]["conversations"], 1);

    spec["group"] = group["key"].clone();
    let evidence = query(spec.clone(), &runs);
    assert_eq!(evidence["primary"]["next_offset"], 1);
    assert_eq!(
        evidence["primary"]["rows"][0]["report_id"],
        "pending-product-route"
    );
    assert_eq!(evidence["comparison"]["rows"][0]["run_id"], "another-org");
    assert_eq!(evidence["comparison"]["rows"][0]["user_id"], "demo-shopper");
    let path = evidence["primary"]["rows"][0]["inspector_path"]
        .as_str()
        .unwrap();
    assert!(path.contains(
        "&probe=pending%2Dproduct%2Droute&probe_source=request%2Dsource%3Arequest%2Dtype"
    ));
    spec["offset"] = json!(2);
    let last = query(spec, &runs);
    assert_eq!(last["primary"]["rows"][0]["run_id"], "second-conversation");
    assert!(last["primary"]["next_offset"].is_null());
}

#[test]
fn aligns_two_windows_and_keeps_incompatible_probe_setups_separate() {
    let first = sample();
    let mut next = first.clone();
    next.id = "next-day".into();
    next.calls[0].started_at = Some("2026-01-16T00:00:00Z".into());
    let mut changed = next.clone();
    changed.id = "changed-tokenizer".into();
    changed.calls[0]
        .candidate_probes
        .as_mut()
        .unwrap()
        .reports
        .as_mut()
        .unwrap()[0]
        .scores
        .tokenizer_sha256 = "a".repeat(64);
    let mut unoriented = next.clone();
    unoriented.id = "other-prefix".into();
    unoriented.calls[0]
        .candidate_probes
        .as_mut()
        .unwrap()
        .reports
        .as_mut()
        .unwrap()[0]
        .scores
        .assistant_prefix_token_ids[0] += 1;
    let mut other_owner = next.clone();
    other_owner.id = "other-owner".into();
    other_owner.organization_id = Some("other-org".into());

    let runs = [first, changed, unoriented, next, other_owner];
    let thread = json!({"trace_source":"langsmith", "organization_id":"demo-retail", "user_id":"demo-shopper", "conversation_id":"candidate-routing-demo"});
    let spec = json!({
        "version":1, "source_id":"store-name",
        "selection": { "thread": thread, "before":"2026-01-16T00:00:00Z" },
        "compare": { "thread": thread, "after":"2026-01-16T00:00:00Z" },
    });
    let page = query(spec, &runs);
    assert_eq!(page["total_groups"], 3);
    let rows = page["groups"].as_array().unwrap();
    let aligned = rows.iter().find(|row| !row["primary"].is_null()).unwrap();
    assert_eq!(aligned["primary"]["measurements"], 1);
    assert_eq!(aligned["comparison"]["measurements"], 1);
    assert_eq!(aligned["comparison"]["reversals"], 0);
    assert_eq!(
        rows.iter().filter(|row| row["primary"].is_null()).count(),
        2
    );
    assert!(rows
        .iter()
        .all(|row| row["comparison"]["conversations"] == 1));
    assert!(!page.to_string().contains("Background: the store"));
}
