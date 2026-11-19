use nexusweave::{inspect, langsmith};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/langsmith.json")).unwrap()
}

#[test]
fn imports_anchored_calls_and_only_explicit_metadata() {
    let runs = langsmith::import(&fixture()).unwrap();
    assert_eq!(runs.len(), 3);
    let first = &runs[0];
    assert_eq!(first.conversation_id.as_deref(), Some("retail-demo-thread"));
    assert_eq!(first.organization_id.as_deref(), Some("demo-retail"));
    assert_eq!(first.user_id.as_deref(), Some("demo-shopper"));
    assert_eq!(runs[1].conversation_id, first.conversation_id);
    assert!(runs[2].conversation_id.is_none());
    assert!(runs[2].calls[0].context.is_empty());
    assert!(runs[2].calls[0].decisions.is_empty());

    let call = &first.calls[0];
    assert_eq!(call.model.as_deref(), Some("synthetic-model"));
    assert_eq!(call.context[0].role.as_deref(), Some("system"));
    assert_eq!(
        call.context[0].evidence.field,
        "inputs.messages[0][0].kwargs.content"
    );
    assert_eq!(
        call.decisions[0].evidence.field,
        "outputs.generations[0][0].message.kwargs.tool_calls[0]"
    );
    assert_eq!(call.decisions[0].outcome, json!({"days":19}));
    assert_eq!(
        runs[1].calls[0].context[0].evidence.field,
        "inputs.messages[0][0].data.content"
    );

    let report = inspect(&runs).unwrap();
    assert_eq!(report.tool_links.len(), 1);
    let link = &report.tool_links[0];
    assert_eq!(link.result_call_id, first.calls[1].id);
    assert_eq!(link.result.evidence.field, "outputs");
    assert_eq!(link.result.outcome, Some(json!({"eligible":true})));
    assert!(first.calls[1].context.is_empty());
}

#[test]
fn rejects_conflicting_thread_identity_and_ambiguous_batches() {
    let mut document = fixture();
    document["runs"][4]["extra"]["metadata"]["thread_id"] = json!("other-thread");
    assert!(langsmith::import(&document)
        .unwrap_err()
        .contains("conflicting thread"));
    let mut document = fixture();
    document["runs"][4]["inputs"]["messages"] = json!([
        [{"role":"user","content":"One"}],
        [{"role":"user","content":"Two"}]
    ]);
    assert!(langsmith::import(&document)
        .unwrap_err()
        .contains("batched inputs.messages"));
}

#[test]
fn matches_openai_calls_and_anthropic_native_results() {
    let document = json!([
        {"id":"c1","trace_id":"root","run_type":"llm","start_time":"2026-11-19T12:00:00Z",
         "outputs":{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[
             {"id":"tool-1","function":{"name":"calculate","arguments":"{}"}}
         ]}}]}},
        {"id":"c2","trace_id":"root","run_type":"llm","start_time":"2026-11-19T12:01:00Z",
         "inputs":{"messages":[{"role":"user","content":[
             {"type":"tool_result","tool_use_id":"tool-1","content":"42"}
         ]}]},"outputs":{"output":"42"}}
    ]);
    let runs = langsmith::import(&document).unwrap();
    let report = inspect(&runs).unwrap();
    let link = &report.tool_links[0];
    assert_eq!(link.result_call_id, "c2");
    assert_eq!(
        runs[0].calls[0].decisions[0].evidence.field,
        "outputs.choices[0].message.tool_calls[0]"
    );
    assert_eq!(link.result.evidence.field, "inputs.messages[0].content[0]");
    assert_eq!(link.result.outcome, Some(json!("42")));
    assert_eq!(
        runs[0].calls[1].decisions[0].evidence.field,
        "outputs.output"
    );
}
