use nexusweave::{
    langsmith,
    replay::{shadow::patterns, MatchRule, Trial},
    Run,
};
use serde_json::{json, Value};

const COHORT: &str =
    include_str!("../examples/runtime-comparison/pending-routing/shadow-cohort.json");

fn cohort() -> Vec<Run> {
    langsmith::import(&serde_json::from_str(COHORT).unwrap()).unwrap()
}

fn attached(run: &mut Run) -> &mut nexusweave::replay::shadow::Report {
    &mut run.calls[0]
        .shadow_probes
        .as_mut()
        .unwrap()
        .reports
        .as_mut()
        .unwrap()[0]
}

fn query(spec: Value, runs: &[Run], include_labels: bool) -> Value {
    patterns::query(&serde_json::from_value(spec).unwrap(), runs, include_labels).unwrap()
}

#[test]
fn compares_measured_proposal_shifts_by_user_organization_thread_and_time() {
    let runs = cohort();
    let mut spec = json!({
        "version": 1, "source_id": "request-type", "min_conversations": 2, "limit": 1,
        "selection": { "scope": {"kind": "user", "organization_id": "retail-north", "user_id": "routing-demo"} },
        "compare": { "scope": {"kind": "organization", "organization_id": "retail-south"} },
    });
    let page = query(spec.clone(), &runs, true);
    assert_eq!(page["total_groups"], 1);
    let group = &page["groups"][0];
    assert_eq!(group["setup"]["expected_label"], "price");
    assert_eq!(group["setup"]["match_rule"], "kind_and_label");
    assert_eq!(group["primary"]["increased"], 2);
    assert_eq!(group["primary"]["measured_conversations"], 2);
    assert_eq!(group["primary"]["min_difference_pp"], 100.0);
    assert_eq!(group["comparison"]["increased"], 1);
    assert_eq!(group["comparison"]["unchanged"], 1);
    assert_eq!(group["comparison"]["min_difference_pp"], 0.0);
    assert_eq!(group["comparison"]["max_difference_pp"], 100.0);

    spec["group"] = group["key"].clone();
    spec["compare_offset"] = json!(1);
    let evidence = query(spec, &runs, true);
    assert_eq!(evidence["primary"]["next_offset"], 1);
    let occurrence = &evidence["comparison"]["rows"][0];
    assert_eq!(occurrence["conversation_id"], "trace-south-stock-only");
    assert_eq!(occurrence["baseline"]["matched"], 0);
    assert_eq!(occurrence["edited"]["matched"], 0);
    assert_eq!(occurrence["difference_pp"], 0.0);
    assert!(occurrence["inspector_path"].as_str().unwrap().contains(
        "&shadow=routing%2Dsouth%2Dstock%2Donly%2Dshadow&shadow_source=request%2Dsource%3Arequest%2Dtype"
    ));

    let thread = json!({"trace_source":"langsmith", "organization_id":"retail-south", "user_id":"routing-demo", "conversation_id":"trace-south-stock-only"});
    let filters = json!({
        "version": 1, "source_id":"request-type",
        "selection": { "thread": thread, "after": "2026-01-17T12:00:00Z" },
        "compare": { "thread": thread, "before": "2026-01-17T12:00:00Z" },
    });
    let filtered = query(filters, &runs, true);
    assert_eq!(filtered["groups"][0]["primary"]["calls"], 1);
    assert!(filtered["groups"][0]["comparison"].is_null());
}

#[test]
fn groups_by_scored_outcome_fields_and_keeps_target_excerpts_local() {
    let mut runs = cohort();
    runs.truncate(2);
    for (run, product_id) in runs.iter_mut().zip([17, 88]) {
        let report = attached(run);
        report.match_rule = MatchRule::OutcomeFields {
            pointers: vec!["/product_id".into()],
        };
        report.expected.outcome = json!({"product_id": product_id});
    }
    let spec = json!({"version": 1, "source_id": "request-type"});
    let local = query(spec.clone(), &runs, true);
    assert_eq!(local["total_groups"], 2);
    let public = query(spec, &runs, false);
    for (visible, redacted) in local["groups"]
        .as_array()
        .unwrap()
        .iter()
        .zip(public["groups"].as_array().unwrap())
    {
        assert_eq!(visible["primary"]["increased"], 1);
        assert_eq!(visible["setup"]["expected_label"], "price");
        assert!(visible["setup"]["target_excerpt"]["text"].is_string());
        assert_eq!(visible["key"], redacted["key"]);
        assert!(redacted["setup"]["expected_label"].is_null());
        assert!(redacted["setup"]["target_excerpt"].is_null());
    }
    assert_ne!(
        local["groups"][0]["setup"]["target_excerpt"],
        local["groups"][1]["setup"]["target_excerpt"]
    );
}

#[test]
fn preserves_missing_comparisons_and_excludes_tampered_reports() {
    let mut runs = cohort();
    runs.truncate(3);
    let partial = attached(&mut runs[1]);
    let edit = &mut partial.effects[0].result;
    edit.trials[0] = Trial::Failed;
    edit.summary.completed = 0;
    edit.summary.matched = 0;
    edit.difference_pp = None;
    partial.pair.as_mut().unwrap().result.interaction_pp = None;
    partial
        .control_comparison
        .as_mut()
        .unwrap()
        .target_minus_control_pp = None;
    attached(&mut runs[2]).baseline_summary.matched = 1;

    let mut spec = json!({"version": 1, "source_id": "request-type"});
    let page = query(spec.clone(), &runs, false);
    let group = &page["groups"][0];
    assert_eq!(group["primary"]["calls"], 2);
    assert_eq!(group["primary"]["conversations"], 2);
    assert_eq!(group["primary"]["measured_conversations"], 1);
    assert_eq!(group["primary"]["increased"], 1);
    assert_eq!(group["primary"]["unchanged"], 0);
    assert_eq!(group["primary"]["unavailable"], 1);
    assert_eq!(group["primary"]["min_difference_pp"], 100.0);
    spec["min_conversations"] = json!(2);
    assert_eq!(query(spec, &runs, false)["total_groups"], 0);
}
