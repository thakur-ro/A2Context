use a2context::{langsmith, otlp, Run};
use serde_json::{json, Value};

fn openinference() -> Value {
    serde_json::from_str(include_str!("../fixtures/openinference.json")).unwrap()
}

#[test]
fn keeps_captured_wrappers_and_reconnects_a_parent_that_arrives_late() {
    let export = openinference();
    let spans = export["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    let subset = |subset_spans: &[Value]| {
        let mut part = export.clone();
        part["resourceSpans"][0]["scopeSpans"][0]["spans"] = json!(subset_spans);
        part
    };
    let mut archive = otlp::Archive::default();
    let child = archive.prepare(&subset(&spans[1..2])).unwrap();
    archive.commit(child);
    let run = archive.runs().next().unwrap();
    assert_eq!(run.calls.len(), 1);
    assert_eq!(run.spans.len(), 1);
    let parent = run.spans[0].parent_span_id.clone().unwrap();
    assert_eq!(run.spans[0].id, run.calls[0].id);

    let root = archive.prepare(&export).unwrap();
    archive.commit(root);
    let merged = archive.runs().next().unwrap();
    assert_eq!(merged.calls.len(), 5);
    assert_eq!(merged.spans.len(), 8);
    assert_eq!(merged.spans[0].name.as_deref(), Some("Inventory workflow"));
    assert!(merged.spans[0].parent_span_id.is_none());
    let stock = merged.spans.iter().find(|span| span.id == parent).unwrap();
    assert_eq!(stock.name.as_deref(), Some("Stock branch"));
    assert_eq!(
        stock.parent_span_id.as_deref(),
        Some(merged.spans[0].id.as_str())
    );
    assert!(!merged.calls.iter().any(|call| call.id == stock.id));
    assert_eq!(
        serde_json::from_value::<Run>(json!(merged)).unwrap(),
        *merged
    );

    let mut equivalent = export.clone();
    equivalent["resourceSpans"][0]["scopeSpans"][0]["spans"][5]
        .as_object_mut()
        .unwrap()
        .shift_remove("parentSpanId");
    assert!(archive.prepare(&equivalent).unwrap().is_empty());
    let mut conflicting = export.clone();
    conflicting["resourceSpans"][0]["scopeSpans"][0]["spans"][1]["parentSpanId"] =
        json!("0000000000000100");
    assert!(archive
        .prepare(&conflicting)
        .err()
        .unwrap()
        .contains("conflicting copies"));
}

#[test]
fn preserves_langsmith_wrapper_spans_and_explicit_missing_parents() {
    let document = json!([
        {"id":"child","trace_id":"root","parent_run_id":"wrapper","run_type":"llm","outputs":{"text":"Hello"}},
        {"id":"detached","trace_id":"root","parent_run_id":"missing","run_type":"llm","outputs":{"text":"Away"}},
        {"id":"root","trace_id":"root","run_type":"chain","name":"Parent workflow"},
        {"id":"wrapper","trace_id":"root","parent_run_id":"root","run_type":"chain","name":"Branch"}
    ]);
    let runs = langsmith::import(&document).unwrap();
    let run = &runs[0];
    assert_eq!(run.calls.len(), 2);
    assert_eq!(run.spans.len(), 4);
    let find = |id| run.spans.iter().find(|span| span.id == id).unwrap();
    assert_eq!(find("child").parent_span_id.as_deref(), Some("wrapper"));
    assert_eq!(find("wrapper").parent_span_id.as_deref(), Some("root"));
    assert_eq!(find("detached").parent_span_id.as_deref(), Some("missing"));
    assert!(find("root").parent_span_id.is_none());
}
