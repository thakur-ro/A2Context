use a2context::{langsmith, otlp, preflight};
use serde_json::{json, Value};

#[test]
fn legacy_preflight_verifies_the_same_saved_receipt() {
    let modern = include_str!("../examples/exchange-approval/pending.json");
    for input in [
        modern.to_owned(),
        modern.replace("a2context.", "nexusweave."),
    ] {
        let request = serde_json::from_str(&input).unwrap();
        let report = preflight::check(request).unwrap();
        assert!(report.source_coverage.is_some());
        assert_eq!(
            report.receipt.unwrap().fingerprint,
            "a66880f98a5fb4cb6b107e996668e37287556ad5919a6bda084641ba7babcf74"
        );
    }
}

#[test]
fn langsmith_preserves_the_actual_field_name_and_prefers_the_current_spelling() {
    let legacy = include_str!("../examples/preflight-returns-langsmith.json")
        .replace("a2context.", "nexusweave.");
    let mut export: Value = serde_json::from_str(&legacy).unwrap();
    let runs = langsmith::import(&export).unwrap();
    let call = &runs[0].calls[0];
    let captured = call.recorded_request.as_ref().unwrap();
    assert_eq!(
        captured.evidence.field,
        r#"extra.metadata["nexusweave.request.body"]"#
    );
    assert!(call.context_metadata.as_ref().unwrap().issue.is_none());
    let body = r#"{ "model": "nexusweave.example", "messages": [] }"#;
    export["extra"]["metadata"]["a2context.request.body"] = json!(body);
    let runs = langsmith::import(&export).unwrap();
    let captured = runs[0].calls[0].recorded_request.as_ref().unwrap();
    assert_eq!(captured.body, body);
    assert_eq!(
        captured.evidence.field,
        r#"extra.metadata["a2context.request.body"]"#
    );
}

#[test]
fn legacy_otlp_can_be_reopened_retried_and_redacted() {
    let legacy = include_str!("../examples/exchange-approval/exchange.otlp.json")
        .replace("a2context.", "nexusweave.");
    let doc: Value = serde_json::from_str(&legacy).unwrap();
    for content in [true, false] {
        let policy = otlp::CapturePolicy {
            content,
            metadata: &Default::default(),
        };
        let batch = otlp::prepare_stored(&doc, &policy, |_| Ok(Default::default())).unwrap();
        let mut saved = std::collections::BTreeMap::new();
        for (id, trace, run) in batch.into_updates() {
            let run = run.unwrap();
            let call = run.calls.iter().find(|c| c.turn_id.is_some()).unwrap();
            assert!(run.organization_id.is_some());
            assert_eq!(call.recorded_request.is_some(), content);
            if content {
                assert_eq!(
                    call.recorded_request.as_ref().unwrap().evidence.field,
                    "nexusweave.request.body"
                );
            }
            let bytes = serde_json::to_vec(&trace).unwrap();
            let reopened: otlp::StoredTrace = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(reopened.pending_request(&call.id).is_some(), content);
            assert_eq!(
                reopened
                    .normalize(id.strip_prefix("otlp:").unwrap())
                    .unwrap()
                    .unwrap()
                    .calls,
                run.calls
            );
            saved.insert(id, reopened);
        }
        otlp::prepare_stored(&doc, &policy, |id| Ok(saved[id].clone())).unwrap();
    }
}
