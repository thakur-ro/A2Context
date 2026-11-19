use nexusweave::pending;
use serde_json::{json, Value};

fn inspection(input: Value) -> Value {
    pending::inspect(serde_json::from_value(input).unwrap()).unwrap()
}

#[test]
fn inspects_rules_and_editable_locations_without_a_recorded_call() {
    let request: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns.json")).unwrap();
    let view = inspection(json!({ "request": request }));
    assert_eq!(view["preflight"]["findings"][0]["kind"], "conflict");
    assert!(view["preflight"]["receipt"]["fingerprint"].is_string());
    assert_eq!(view["screening"]["status"], "matched");
    assert!(view["screening"]["report"]["scorer"].is_null());
    assert!(view["request_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["fields"][0]["pointer"] == "/messages/0/content"
                && item["fields"][0]["text_range"]["end_byte"].is_number()
        }));
    assert!(view.get("request").is_none());
}

#[test]
fn verifies_pending_model_reports_against_the_supplied_request() {
    let request: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/request.json"
    ))
    .unwrap();
    let candidate: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/report.json"
    ))
    .unwrap();
    let shadow: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/shadow-report.json"
    ))
    .unwrap();
    let mut input = json!({ "request": request, "candidate": candidate, "shadow": shadow });
    let view = inspection(input.clone());
    assert_eq!(view["candidate_probes"][0]["status"], "matched");
    assert_eq!(view["shadow_probes"][0]["status"], "matched");
    assert_eq!(
        view["shadow_probes"][0]["measurement"]["pair"]["difference_pp"],
        100.0
    );
    assert!(view["shadow_probes"][0]["measurement"]["effects"][0]["context_id"].is_string());
    let body = &mut input["request"]["attributes"]["nexusweave.request.body"];
    *body = json!(format!("{} ", body.as_str().unwrap()));
    let stale = inspection(input);
    assert_eq!(stale["candidate_probes"][0]["status"], "unverified");
    assert_eq!(stale["shadow_probes"][0]["status"], "unverified");
    assert!(stale["candidate_probes"][0]["measurement"].is_null());
    assert!(stale["shadow_probes"][0]["measurement"].is_null());
}

#[test]
fn verifies_an_external_screen_against_the_pending_text() {
    let request: Value =
        serde_json::from_str(include_str!("../examples/context-screening/request.json")).unwrap();
    let evidence: Value =
        serde_json::from_str(include_str!("../examples/context-screening/evidence.json")).unwrap();
    let report = &evidence[0]["calls"][0]["context_screening"]["report"];
    let mut input = json!({ "request": request, "screening": report });
    assert_eq!(inspection(input.clone())["screening"]["status"], "matched");
    input["request"]["attributes"]["gen_ai.input.messages"][0]["parts"][0]["content"] =
        json!("Receipts are available to staff only.");
    let stale = inspection(input);
    assert_eq!(stale["screening"]["status"], "unverified");
    assert!(stale["screening"]["report"].is_null());
}
