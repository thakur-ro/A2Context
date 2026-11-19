use nexusweave::{inspect, otlp, Availability};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/otlp.json")).unwrap()
}

#[test]
fn captures_only_explicit_inference_request_bodies_and_preserves_the_raw_json() {
    let mut document = fixture();
    let runs = otlp::import(&document).unwrap();
    assert!(runs[0].calls[0].recorded_request.is_none());
    let spans = document["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array_mut()
        .unwrap();
    let body = r#"{ "model":"stock", "seed":9223372036854775807 }"#;
    let attribute = json!({"key": "nexusweave.request.body", "value": {"stringValue": body}});
    for span in spans.iter_mut().filter(|span| {
        matches!(
            span["spanId"].as_str(),
            Some("0000000000000002" | "0000000000000003")
        )
    }) {
        span["attributes"]
            .as_array_mut()
            .unwrap()
            .push(attribute.clone());
    }
    let runs = otlp::import(&document).unwrap();
    let capture = runs[0].calls[0].recorded_request.as_ref().unwrap();
    assert_eq!(capture.body, body);
    assert_eq!(capture.evidence.field, "nexusweave.request.body");
    assert_eq!(capture.evidence.span_id, runs[0].calls[0].span_id);
    assert!(runs[0].calls[1].recorded_request.is_none());
    let written = serde_json::to_string(&runs).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<nexusweave::Run>>(&written).unwrap(),
        runs
    );

    let span = document["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|span| span["spanId"] == "0000000000000002")
        .unwrap();
    span["attributes"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["value"]["stringValue"] = json!("[]");
    let runs = otlp::import(&document).unwrap();
    assert!(runs[0].calls[0].recorded_request.is_none());
    assert_eq!(runs[0].calls[0].decisions[0].label, "look_up_inventory");
}

#[test]
fn imports_genai_spans_and_links_tool_evidence_in_recorded_order() {
    let runs = otlp::import(&fixture()).unwrap();
    assert_eq!(runs.len(), 2);
    let trace = &runs[0];
    assert_eq!(trace.calls.len(), 3);
    assert_eq!(trace.organization_id.as_deref(), Some("demo-retail"));
    assert_eq!(trace.user_id.as_deref(), Some("inventory-user"));
    assert_eq!(
        trace.conversation_id.as_deref(),
        Some("inventory-demo-thread")
    );
    assert_eq!(runs[1].conversation_id, trace.conversation_id);

    let first = &trace.calls[0];
    assert_eq!(first.id, "0000000000000002");
    assert_eq!(first.turn_id.as_deref(), Some("inventory-turn-1"));
    assert_eq!(
        first.started_at.as_deref(),
        Some("2026-11-19T13:00:00.000000002Z")
    );
    assert_eq!(first.context.len(), 3);
    assert_eq!(first.context[0].evidence.field, "gen_ai.input.messages");
    assert_eq!(
        first.context[1].evidence.field,
        "gen_ai.system_instructions"
    );
    assert_eq!(first.decisions[0].outcome, json!({"sku":"blue-mug"}));

    let report = inspect(&runs).unwrap();
    assert_eq!(report.tool_links.len(), 1);
    let link = &report.tool_links[0];
    assert_eq!(link.result_call_id, "0000000000000003");
    assert_eq!(link.result.evidence.field, "gen_ai.tool.call.result");
    assert_eq!(
        link.result.outcome,
        Some(json!({"in_stock":true,"quantity":42}))
    );
    assert!(trace.calls[1].context.is_empty());
    assert_eq!(trace.calls[2].context[0].kind, "tool_call_response");
    assert_eq!(
        trace.calls[2].decisions[0].outcome,
        json!("That item is available.")
    );
}

#[test]
fn leaves_content_references_unread_and_records_masked_tool_results() {
    let mut document = fixture();
    let tool = &mut document["resourceSpans"][0]["scopeSpans"][0]["spans"][1];
    tool["attributes"]
        .as_array_mut()
        .unwrap()
        .retain(|attribute| attribute["key"] != "gen_ai.tool.call.result");
    let runs = otlp::import(&document).unwrap();
    let external = &runs[1].calls[0].context[1];
    assert_eq!(external.availability, Availability::Unknown);
    assert_eq!(external.text, None);
    assert_eq!(external.evidence.field, "gen_ai.input.messages_ref");
    assert!(runs[1].calls[0].decisions.is_empty());
    let report = inspect(&runs).unwrap();
    assert_eq!(
        report.tool_links[0].result.outcome,
        Some(json!({"in_stock":true}))
    );
    assert_eq!(
        report.tool_links[0].result.evidence.field,
        "gen_ai.input.messages"
    );
    assert_eq!(runs[0].calls[1].decisions[0].outcome, Value::Null);
    assert_eq!(
        runs[0].calls[1].decisions[0].evidence.field,
        "gen_ai.tool.name"
    );

    let mut masked = runs;
    masked[0].calls[2].tool_results.clear();
    let report = inspect(&masked).unwrap();
    assert_eq!(report.tool_links[0].result.outcome, None);
    assert_eq!(
        report.tool_links[0].result.evidence.field,
        "gen_ai.tool.call.id"
    );
}

#[test]
fn accepts_duplicate_batches_and_rejects_conflicting_or_malformed_records() {
    let batch = fixture();
    let twice = json!([batch.clone(), batch.clone()]);
    assert_eq!(otlp::import(&twice).unwrap(), otlp::import(&batch).unwrap());
    let mut conflict = batch.clone();
    conflict["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"] = json!("different");
    assert!(otlp::import(&json!([batch.clone(), conflict]))
        .unwrap_err()
        .contains("conflicting copies"));
    let mut bad_id = batch.clone();
    bad_id["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"] = json!("not-an-otlp-id");
    assert!(otlp::import(&bad_id)
        .unwrap_err()
        .contains("invalid OTLP traceId"));
}

#[test]
fn merges_out_of_order_batches_and_rejects_an_inconsistent_update_atomically() {
    let document = fixture();
    let expected = otlp::import(&document).unwrap();
    let first = &document["resourceSpans"][0];
    let spans = first["scopeSpans"][0]["spans"].as_array().unwrap();
    let spans: Vec<_> = spans
        .iter()
        .filter(|span| span["traceId"] == spans[0]["traceId"])
        .collect();
    let subset = |index: usize| {
        let mut part = first.clone();
        part["scopeSpans"][0]["spans"] = json!([spans[index]]);
        json!({"resourceSpans": [part]})
    };
    let mut archive = otlp::Archive::default();
    for index in (0..spans.len()).rev() {
        let part = subset(index);
        let batch = archive.prepare(&part).unwrap();
        assert!(!batch.is_empty());
        archive.commit(batch);
        assert!(archive.prepare(&part).unwrap().is_empty());
    }
    assert_eq!(archive.runs().collect::<Vec<_>>(), [&expected[0]]);
    let mut bad = subset(0);
    bad["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"] = json!("conflicting copy");
    assert!(archive.prepare(&json!([document, bad])).is_err());
    assert_eq!(archive.runs().collect::<Vec<_>>(), [&expected[0]]);
    let remaining = archive.prepare(&document).unwrap();
    archive.commit(remaining);
    assert_eq!(archive.runs().cloned().collect::<Vec<_>>(), expected);
}
