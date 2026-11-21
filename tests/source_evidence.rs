use a2context::{langsmith, source_profiles, Run};
use serde_json::{json, Value};

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
