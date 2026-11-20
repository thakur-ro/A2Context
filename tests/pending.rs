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

#[test]
fn binds_first_token_ranking_to_the_same_edits_as_the_tool_proposals() {
    let request: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/prefill-preview.json"
    ))
    .unwrap();
    let shadow: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/shadow-report.json"
    ))
    .unwrap();
    let attr = nexusweave::replay::prefill::ATTRIBUTE;
    let envelope = json!({"request":request});
    let view = inspection(envelope.clone());
    let ranked = view["prefill"]["ranked"].as_array().unwrap();
    assert_eq!(ranked.len(), 2);
    assert!(ranked[0]["js_bits"].as_f64().unwrap() > ranked[1]["js_bits"].as_f64().unwrap());
    for (row, effect) in ranked.iter().zip(shadow["effects"].as_array().unwrap()) {
        assert_eq!(row["context_id"], effect["intervention"]["context_id"]);
        assert_eq!(row["request_sha256"], effect["request_sha256"]);
    }
    let evidence = pending::evidence(
        serde_json::from_value(json!({"input":envelope})).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        evidence["prefill"]["ranked"][0]["js_bits"],
        ranked[0]["js_bits"]
    );
    assert!(evidence["prefill"]["ranked"][0]
        .get("replacement")
        .is_none());
    assert_eq!(ranked[0]["replacement"], "Request type: price.");
    assert!(!evidence["prefill"]
        .to_string()
        .contains("Request type: price."));

    for (field, value) in [
        ("js_bits", json!(-0.1)),
        ("js_bits", json!(1.1)),
        ("request_sha256", json!("wrong")),
        ("context_id", json!("other")),
        ("source_version", json!("other")),
    ] {
        let mut input = envelope.clone();
        input["request"]["attributes"][attr]["scores"]["effects"][0][field] = value;
        assert!(pending::inspect(serde_json::from_value(input).unwrap()).is_err());
    }
    let mut swapped = envelope.clone();
    swapped["request"]["attributes"][attr]["interventions"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    assert!(pending::inspect(serde_json::from_value(swapped).unwrap()).is_err());
    let mut raw = envelope;
    let attributes = &mut raw["request"]["attributes"];
    let mut body: Value =
        serde_json::from_str(attributes["nexusweave.request.body"].as_str().unwrap()).unwrap();
    body["temperature"] = json!(0.8);
    attributes["nexusweave.request.body"] = json!(body.to_string());
    assert!(pending::inspect(serde_json::from_value(raw).unwrap()).is_err());
}

#[test]
fn opening_tokens_require_complete_pairs_and_follow_excerpt_permissions() {
    let original: Value = serde_json::from_str(include_str!(
        "../examples/context-screening/replay/opening-followup.json"
    ))
    .unwrap();
    let attr = nexusweave::replay::prefill::ATTRIBUTE;
    let scores = &original["request"]["attributes"][attr]["scores"];
    let view = inspection(original.clone());
    assert_eq!(view["prefill"]["top_token_id"], scores["top_token_id"]);
    for row in view["prefill"]["ranked"].as_array().unwrap() {
        assert_eq!(row["opening_changed"], false);
        assert_eq!(row["top_token_id"], scores["top_token_id"]);
    }
    let mut input = original.clone();
    let effect = &mut input["request"]["attributes"][attr]["scores"]["effects"][0];
    effect["top_token_id"] = json!(scores["top_token_id"].as_u64().unwrap() + 1);
    let evidence = pending::evidence(
        serde_json::from_value(json!({"input":input})).unwrap(),
        false,
    )
    .unwrap();
    assert!(evidence["prefill"].get("top_token_id").is_none());
    let rows = evidence["prefill"]["ranked"].as_array().unwrap();
    assert!(rows.iter().all(|row| row.get("top_token_id").is_none()));
    assert_eq!(
        rows.iter()
            .filter(|row| row["opening_changed"] == true)
            .count(),
        1
    );
    for field in ["baseline", "edit"] {
        let mut input = original.clone();
        let scores = &mut input["request"]["attributes"][attr]["scores"];
        let owner = if field == "baseline" {
            scores
        } else {
            &mut scores["effects"][0]
        };
        owner.as_object_mut().unwrap().remove("top_token_id");
        assert!(pending::inspect(serde_json::from_value(input).unwrap()).is_err());
    }
}

#[test]
fn prefill_validates_all_edits_before_calling_the_adapter() {
    use nexusweave::replay::prefill;
    let raw: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/prefill-preview.json"
    ))
    .unwrap();
    let probe = &raw["attributes"][prefill::ATTRIBUTE];
    let input = || serde_json::from_value(raw.clone()).unwrap();
    let edits = &probe["interventions"];
    let result = prefill::measure(
        input(),
        serde_json::from_value(edits.clone()).unwrap(),
        |batch| {
            assert_eq!(
                batch["baseline_request_sha256"],
                probe["scores"]["baseline_request_sha256"]
            );
            for (edit, effect) in batch["edits"]
                .as_array()
                .unwrap()
                .iter()
                .zip(probe["scores"]["effects"].as_array().unwrap())
            {
                assert_eq!(edit["request_sha256"], effect["request_sha256"]);
                assert_eq!(edit["source_id"], effect["source_id"]);
            }
            Ok(serde_json::from_value(probe["scores"].clone()).unwrap())
        },
    )
    .unwrap();
    assert_eq!(result.attributes[prefill::ATTRIBUTE], *probe);
    let mut invalid = edits.clone();
    invalid[1]["pointer"] = json!("/messages/0/content");
    for cases in [json!([]), json!([edits[0], edits[0]]), invalid] {
        assert!(
            prefill::measure(input(), serde_json::from_value(cases).unwrap(), |_| panic!(
                "invalid plan invoked scorer"
            ))
            .is_err()
        );
    }
}
