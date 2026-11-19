use nexusweave::{langsmith, replay::Trial, source_profiles, Run};
use serde_json::{json, Value};

const PROBE_TRACE: &str =
    include_str!("../examples/runtime-comparison/pending-routing/shadow-trace.json");
const CANDIDATE_TRACE: &str =
    include_str!("../examples/runtime-comparison/pending-routing/trace.json");
const CONFLICT_TRACE: &str = include_str!("../examples/preflight-returns-langsmith.json");

fn source_query(request: Value, runs: &[Run]) -> Value {
    source_profiles::query::query(
        &serde_json::from_value(request).unwrap(),
        runs.iter(),
        &[],
        &[],
    )
    .unwrap()
}

fn probe_trace() -> Run {
    let mut trace: Value = serde_json::from_str(PROBE_TRACE).unwrap();
    let candidate: Value = serde_json::from_str(CANDIDATE_TRACE).unwrap();
    let key = "nexusweave.candidate_probes";
    trace["extra"]["metadata"][key] = candidate["extra"]["metadata"][key].clone();
    langsmith::import(&trace).unwrap().remove(0)
}

#[test]
fn connects_valid_probes_to_the_same_source_occurrence_and_hides_trial_content() {
    let run = probe_trace();
    let mut stale = run.clone();
    stale.id = "stale-probe".into();
    stale.organization_id = Some("other-org".into());
    stale.calls[0]
        .recorded_request
        .as_mut()
        .unwrap()
        .body
        .push(' ');
    let runs = [run, stale];
    let mut spec = json!({
        "version": 1, "source_id": "request-type",
        "scope": {"kind":"organization","organization_id":"demo-retail"},
        "compare": {"scope":{"kind":"organization","organization_id":"other-org"}}
    });
    let compared = source_query(spec.clone(), &runs);
    let primary = &compared["scope_a"];
    assert_eq!(primary["source"]["candidate_probed_occurrences"], 1);
    assert_eq!(primary["source"]["shadow_probed_occurrences"], 1);
    assert_eq!(primary["source"]["measured_occurrences"], 1);
    assert_eq!(compared["scope_b"]["source"]["measured_occurrences"], 0);
    let occurrence = &primary["rows"][0];
    assert_eq!(occurrence["shadow_measurement_count"], 1);
    assert_eq!(occurrence["shadow_example"]["difference_pp"], 100.0);
    assert!(occurrence["shadow_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .contains("&shadow=pending%2Dproduct%2Droute%2Dshadow&shadow_source=request%2Dsource%3Arequest%2Dtype"));
    assert_eq!(occurrence["candidate_measurement_count"], 1);
    assert!(occurrence["candidate_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .contains(
            "&probe=pending%2Dproduct%2Droute&probe_source=request%2Dsource%3Arequest%2Dtype"
        ));
    assert!(!compared.to_string().contains("Request type: price."));
    assert!(compared["scope_b"]["rows"][0]
        .get("shadow_example")
        .is_none());

    spec["evidence_filter"] = "measured".into();
    let measured = source_query(spec.clone(), &runs);
    assert_eq!(measured["scope_a"]["total_occurrences"], 1);
    assert!(measured["scope_b"].is_null());
    spec["evidence_filter"] = "unmeasured".into();
    let unmeasured = source_query(spec, &runs);
    assert!(unmeasured["scope_a"].is_null());
    assert_eq!(unmeasured["scope_b"]["total_occurrences"], 1);
}

#[test]
fn distinguishes_incomplete_shadow_trials_from_a_measured_zero() {
    let mut run = probe_trace();
    let call = &mut run.calls[0];
    call.candidate_probes = None;
    let report = &mut call
        .shadow_probes
        .as_mut()
        .unwrap()
        .reports
        .as_mut()
        .unwrap()[0];
    let incomplete = &mut report.effects[0].result;
    incomplete.trials[0] = Trial::Failed;
    incomplete.summary.completed = 0;
    incomplete.summary.matched = 0;
    incomplete.difference_pp = None;
    report.pair.as_mut().unwrap().result.interaction_pp = None;
    report
        .control_comparison
        .as_mut()
        .unwrap()
        .target_minus_control_pp = None;
    let runs = [run];

    let incomplete = source_query(
        json!({
            "version":1, "source_id":"request-type", "evidence_filter":"unmeasured"
        }),
        &runs,
    );
    assert_eq!(incomplete["source"]["shadow_unavailable_occurrences"], 1);
    assert_eq!(incomplete["source"]["shadow_probed_occurrences"], 0);
    assert_eq!(incomplete["rows"][0]["shadow_unavailable_count"], 1);
    assert!(incomplete["rows"][0]["shadow_example"]["difference_pp"].is_null());
    assert!(incomplete["rows"][0]["shadow_example"]["inspector_path"].is_string());

    let zero = source_query(
        json!({
            "version":1, "source_id":"store-name", "evidence_filter":"measured"
        }),
        &runs,
    );
    assert_eq!(zero["source"]["shadow_probed_occurrences"], 1);
    assert_eq!(zero["rows"][0]["shadow_example"]["difference_pp"], 0.0);
}

#[test]
fn counts_one_conflicted_occurrence_across_preflight_and_decision() {
    let run = langsmith::import(&serde_json::from_str(CONFLICT_TRACE).unwrap())
        .unwrap()
        .remove(0);
    let mut receipt_only = run.clone();
    receipt_only.id = "receipt-only".into();
    receipt_only.calls[0].started_at = Some("2026-01-16T12:00:00Z".into());
    receipt_only.calls[0].decisions.clear();
    let mut decision_only = run.clone();
    decision_only.id = "decision-only".into();
    decision_only.calls[0].started_at = Some("2026-01-17T12:00:00Z".into());
    decision_only.calls[0]
        .recorded_request
        .as_mut()
        .unwrap()
        .body
        .push(' ');
    let mut unknown = decision_only.calls[0]
        .context
        .iter()
        .find(|item| item.source_id.as_deref() == Some("retail/archive-faq"))
        .unwrap()
        .clone();
    unknown.id = "unknown".into();
    unknown.source_id = Some("unknown".into());
    unknown.rule.as_mut().unwrap().value = None;
    decision_only.calls[0].context.push(unknown);
    let runs = [run, receipt_only, decision_only];
    let result = source_query(
        json!({
            "version":1, "source_id":"retail/returns-policy", "evidence_filter":"conflicts"
        }),
        &runs,
    );
    assert_eq!(result["source"]["conflicted_occurrences"], 3);
    assert_eq!(result["source"]["preflight_conflicted_occurrences"], 2);
    assert_eq!(result["source"]["decision_conflicted_occurrences"], 2);
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(
        rows[0]["preflight_conflict_example"]["opposing_source_id"],
        "retail/archive-faq"
    );
    assert!(rows[0]["preflight_conflict_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&preflight_source=request%2Dsource%3Apolicy"));
    assert!(rows[0]["decision_conflict_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .contains("&finding=%5B%22conflict%22%2C%22return%5Fwindow%5Fdays"));
    assert_eq!(rows[1]["decision_conflict_count"], 0);
    assert!(rows[1].get("decision_conflict_example").is_none());
    assert_eq!(rows[2]["preflight_conflict"], false);
    assert_eq!(rows[2]["decision_conflict_count"], 1);

    let excluded = source_query(
        json!({
            "version":1, "source_id":"unknown", "evidence_filter":"conflicts"
        }),
        &runs,
    );
    assert_eq!(excluded["total_occurrences"], 0);
}
