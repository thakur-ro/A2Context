use nexusweave::{document_handoffs::HandoffStatus, inspect, otlp, replay, Availability};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/retrieval.json")).unwrap()
}

fn spans(export: &mut Value) -> &mut Vec<Value> {
    export["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array_mut()
        .unwrap()
}

#[test]
fn connects_only_the_document_recorded_at_inference_and_retains_exact_evidence() {
    let runs = otlp::import(&fixture()).unwrap();
    let run = &runs[0];
    let retriever = &run.calls[0];
    let model = &run.calls[1];
    let documents = &retriever.retrieval.as_ref().unwrap().documents;
    assert_eq!(documents.len(), 2);
    assert_eq!(retriever.context[0].kind, "retrieval_query");
    assert!(retriever.decisions.is_empty());
    assert_eq!(
        documents[0]
            .document_identity
            .as_ref()
            .unwrap()
            .evidence
            .field,
        "retrieval.documents.0.document.id"
    );
    assert!(retriever.ended_at.is_some());
    let input = &model.context[1];
    assert_eq!(
        input.document_identity.as_ref().unwrap().id,
        "policy-current"
    );
    assert_eq!(input.source_id.as_deref(), Some("retail/returns-policy"));
    assert!(input.evidence.field.ends_with(".document.content"));
    let inspection = inspect(&runs).unwrap();
    let [handoff] = &inspection.document_handoffs[..] else {
        panic!("expected one matched input");
    };
    assert_eq!(handoff.call_id, model.id);
    assert_eq!(handoff.context_id, input.id);
    assert_eq!(handoff.status, HandoffStatus::Matched);
    let origin = handoff.origin.unwrap();
    assert_eq!(origin.call_id, retriever.id);
    assert_eq!(origin.document_id, documents[0].id);

    let body: Value = serde_json::from_str(&model.recorded_request.as_ref().unwrap().body).unwrap();
    let case: replay::Case = serde_json::from_value(json!({
        "version":1,"id":"test-return-window","runner_id":"Synthetic support","location":{
            "run_id":run.id,"call_id":model.id,"decision_id":model.decisions[0].id
        },
        "request":body,"samples":2,"interventions":[{
            "context_id":input.id,
            "pointer":"/messages/1/content/0/document/content",
            "replacement":"Returns are accepted within 60 days."
        }]
    }))
    .unwrap();
    let replay = replay::execute(&case, &runs, |request| {
        let content = request
            .pointer("/messages/1/content/0/document/content")
            .unwrap()
            .as_str()
            .unwrap();
        Some(replay::Choice {
            kind: "model_output".into(),
            label: "Model answer".into(),
            outcome: if content.contains("60") {
                json!("60 days")
            } else {
                json!("30 days")
            },
        })
    })
    .unwrap();
    assert_eq!(replay.variants[0].difference_pp, Some(-100.0));
}

#[test]
fn reports_ambiguous_changed_masked_and_untimed_matches_without_inventing_an_origin() {
    for (content, omit_end, duplicate, status) in [
        (
            "Returns are accepted within 30 days.",
            true,
            false,
            HandoffStatus::TimingUnavailable,
        ),
        (
            "Returns are accepted within 30 days.",
            false,
            true,
            HandoffStatus::Ambiguous,
        ),
        (
            "Returns are accepted within 90 days.",
            false,
            false,
            HandoffStatus::ContentDiffers,
        ),
        (
            "__REDACTED__",
            false,
            false,
            HandoffStatus::ContentUnavailable,
        ),
    ] {
        let mut export = fixture();
        let spans = spans(&mut export);
        let source = &mut spans[1];
        let attributes = source["attributes"].as_array_mut().unwrap();
        let entry = attributes
            .iter_mut()
            .find(|entry| entry["key"] == "retrieval.documents.0.document.content")
            .unwrap();
        entry["value"]["stringValue"] = json!(content);
        if omit_end {
            source
                .as_object_mut()
                .unwrap()
                .shift_remove("endTimeUnixNano");
        }
        if duplicate {
            let mut duplicate = source.clone();
            duplicate["spanId"] = json!("0000000000000013");
            spans.push(duplicate);
        }
        let runs = otlp::import(&export).unwrap();
        let inspection = inspect(&runs).unwrap();
        assert_eq!(inspection.document_handoffs[0].status, status);
        assert!(inspection.document_handoffs[0].origin.is_none());
        if content == "__REDACTED__" {
            let item = &runs[0].calls[0].retrieval.as_ref().unwrap().documents[0];
            assert_eq!(item.availability, Availability::Redacted);
            assert!(item.text.is_none());
        }
    }
}

#[test]
fn ignores_retrievals_that_finish_after_inference_starts_or_belong_to_another_trace() {
    let mut export = fixture();
    let entries = spans(&mut export);
    entries[1]["endTimeUnixNano"] = entries[2]["endTimeUnixNano"].clone();
    let runs = otlp::import(&export).unwrap();
    assert!(inspect(&runs).unwrap().document_handoffs.is_empty());

    let mut export = fixture();
    spans(&mut export)[1]["traceId"] = json!("a69d0af2445e46958e08977de02f61a5");
    let runs = otlp::import(&export).unwrap();
    assert_eq!(runs.len(), 2);
    assert!(inspect(&runs).unwrap().document_handoffs.is_empty());
}

#[test]
fn matches_span_level_documents_with_numeric_ids() {
    let mut export = fixture();
    let spans = spans(&mut export);
    let source = spans[1]["attributes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|attribute| attribute["key"] == "retrieval.documents.0.document.id")
        .unwrap();
    source["value"] = json!({"intValue": "4007"});
    let input = spans[2]["attributes"].as_array_mut().unwrap();
    input.retain(|attribute| {
        let field = attribute["key"].as_str().unwrap();
        !field.starts_with("llm.input_messages.1.") && field != "nexusweave.context.semantics"
    });
    input.push(json!({"key":"input.documents.7.document.id","value":{"stringValue":"4007"}}));
    input.push(json!({"key":"input.documents.7.document.content","value":{"stringValue":"Returns are accepted within 30 days."}}));
    let runs = otlp::import(&export).unwrap();
    let report = inspect(&runs).unwrap();
    assert_eq!(report.document_handoffs[0].status, HandoffStatus::Matched);
    assert_eq!(
        report.document_handoffs[0].context_id,
        "input.documents.7.document.content"
    );
}
