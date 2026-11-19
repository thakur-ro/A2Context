use nexusweave::{inspect, otlp, replay, Availability};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/openinference.json")).unwrap()
}

fn attrs(document: &mut Value, span: usize) -> &mut Vec<Value> {
    document["resourceSpans"][0]["scopeSpans"][0]["spans"][span]["attributes"]
        .as_array_mut()
        .unwrap()
}

fn set(attrs: &mut Vec<Value>, key: &str, value: &str) {
    let entry = json!({"key":key,"value":{"stringValue":value}});
    match attrs.iter_mut().find(|attribute| attribute["key"] == key) {
        Some(attribute) => *attribute = entry,
        None => attrs.push(entry),
    }
}

#[test]
fn preserves_openinference_routing_tool_handoffs_and_guardrail_results() {
    let runs = otlp::import(&fixture()).unwrap();
    let run = &runs[0];
    assert_eq!(
        run.conversation_id.as_deref(),
        Some("openinference-inventory")
    );
    assert_eq!(run.user_id.as_deref(), Some("inventory-user"));
    assert_eq!(run.organization_id.as_deref(), Some("demo-retail"));
    let [router, tool_choice, tool, reply, guardrail] = &run.calls[..] else {
        panic!("expected the full workflow");
    };
    assert_eq!(router.model.as_deref(), Some("router-v1"));
    assert_eq!(router.decisions[0].kind, "decision_model");
    assert_eq!(router.decisions[0].label, "Choose support route");
    assert_eq!(router.decisions[0].outcome["route"], "stock");
    assert_eq!(router.decisions[0].evidence.field, "output.value");
    assert_eq!(
        router.context[0].source_id.as_deref(),
        Some("support-routing-state")
    );
    assert_eq!(router.context[0].evidence.field, "input.value#/state");
    assert!(router.context_metadata.as_ref().unwrap().issue.is_none());

    assert_eq!(tool_choice.decisions[0].label, "look_up_inventory");
    assert_eq!(tool_choice.decisions[0].outcome, json!({"sku":"MUG-42"}));
    assert_eq!(
        tool_choice.decisions[0].evidence.field,
        "llm.output_messages.0.message.tool_calls.0.tool_call.function.arguments"
    );
    let report = inspect(&runs).unwrap();
    assert!(report
        .context_links
        .iter()
        .any(|link| link.context_id == "llm.tools.0.tool.json_schema"));
    assert!(report
        .context_links
        .iter()
        .any(|link| link.context_id == "llm.input_messages.1.message.content"));
    assert_eq!(report.tool_links[0].result_call_id, tool.id);
    assert_eq!(
        report.tool_links[0].result.outcome,
        Some(json!({"sku":"MUG-42","quantity":42}))
    );
    assert_eq!(reply.context[0].kind, "tool_call_response");
    assert_eq!(reply.tool_results[0].tool_call_id, "stock-1");
    assert_eq!(
        reply.tool_results[0].evidence.field,
        reply.context[0].evidence.field
    );
    assert_eq!(guardrail.decisions[0].kind, "guardrail");
    assert_eq!(guardrail.decisions[0].outcome, json!({"allowed":true}));
}

#[test]
fn replays_a_structured_decision_against_its_captured_request() {
    let runs = otlp::import(&fixture()).unwrap();
    let run = &runs[0];
    let call = &run.calls[0];
    let request: Value =
        serde_json::from_str(&call.recorded_request.as_ref().unwrap().body).unwrap();
    let case: replay::Case = serde_json::from_value(json!({
        "version":1, "id":"router-test", "runner_id":"Synthetic router", "samples":2, "request":request,
        "location":{"run_id":run.id,"call_id":call.id,"decision_id":call.decisions[0].id},
        "match_rule":{"kind":"outcome_fields","pointers":["/route"]},
        "interventions":[{"context_id":"input.value#/state","pointer":"/state","operation":"replace_json",
                          "replacement":"{\"department\":\"returns\",\"sku\":\"MUG-42\"}"}]
    })).unwrap();
    let report = replay::execute(&case, &runs, |request| {
        let route = if request["state"]["department"] == "inventory" {
            "stock"
        } else {
            "returns"
        };
        Some(replay::Choice {
            kind: "decision_model".into(),
            label: "Choose support route".into(),
            outcome: json!({"route":route,"confidence":0.65}),
        })
    })
    .unwrap();
    assert_eq!(report.baseline_summary.matched, 2);
    assert_eq!(report.variants[0].difference_pp, Some(-100.0));
    replay::validate_report(&report, &runs).unwrap();
}

#[test]
fn preserves_numeric_part_order_and_honors_masked_context_and_outcomes() {
    let mut export = fixture();
    let chat = attrs(&mut export, 1);
    set(chat, "llm.input_messages.0.message.content", "__REDACTED__");
    set(chat, "user.id", "__REDACTED__");
    for field in chat.iter_mut() {
        let key = field["key"].as_str().unwrap();
        if let Some(suffix) = key.strip_prefix("llm.output_messages.0.message.tool_calls.0.") {
            field["key"] = json!(format!("llm.output_messages.0.message.contents.2.{suffix}"));
        }
    }
    set(
        chat,
        "llm.output_messages.0.message.contents.2.message_content.type",
        "tool_use",
    );
    set(
        chat,
        "llm.output_messages.0.message.contents.10.message_content.text",
        "Answer after tool part",
    );
    set(
        chat,
        "llm.output_messages.0.message.contents.11.message_content.type",
        "reasoning",
    );
    set(
        chat,
        "llm.output_messages.0.message.contents.11.message_content.text",
        "Recorded reasoning summary",
    );
    set(chat, "llm.tools.0.tool.name", "look_up_inventory");
    chat.retain(|attribute| attribute["key"] != "llm.tools.0.tool.json_schema");
    set(attrs(&mut export, 2), "output.value", "__REDACTED__");
    let runs = otlp::import(&export).unwrap();
    let chat = &runs[0].calls[1];
    assert_eq!(chat.context[0].availability, Availability::Redacted);
    assert!(chat.context[0].text.is_none());
    assert_eq!(chat.decisions.len(), 2);
    assert_eq!(chat.decisions[0].kind, "tool_call");
    assert_eq!(chat.decisions[1].outcome, "Answer after tool part");
    assert_eq!(runs[0].calls[2].decisions[0].outcome, Value::Null);
    assert!(runs[0].calls[2].tool_results[0].outcome.is_none());
    assert_eq!(runs[0].user_id.as_deref(), Some("inventory-user"));
    let report = inspect(&runs).unwrap();
    assert!(report
        .context_links
        .iter()
        .any(|link| link.context_id == "llm.tools.0.tool.name"));
    assert_eq!(report.tool_links[0].result_call_id, runs[0].calls[3].id);
}

#[test]
fn normalizes_dual_written_genai_spans_once() {
    let mut export = fixture();
    let chat = attrs(&mut export, 1);
    set(chat, "gen_ai.operation.name", "chat");
    set(
        chat,
        "gen_ai.input.messages",
        r#"[{"role":"user","parts":[{"type":"text","content":"Choose a tool."}]}]"#,
    );
    set(
        chat,
        "gen_ai.output.messages",
        r#"[{"role":"assistant","parts":[{"type":"tool_call","name":"look_up_inventory","id":"stock-1","arguments":{"sku":"MUG-42"}}]}]"#,
    );
    let runs = otlp::import(&export).unwrap();
    let call = &runs[0].calls[1];
    assert_eq!(call.context.len(), 1);
    assert_eq!(call.decisions.len(), 1);
    assert_eq!(call.decisions[0].evidence.field, "gen_ai.output.messages");
    assert_eq!(
        runs[0].conversation_id.as_deref(),
        Some("openinference-inventory")
    );
    attrs(&mut export, 1).retain(|entry| {
        !matches!(
            entry["key"].as_str(),
            Some("gen_ai.input.messages" | "gen_ai.output.messages")
        )
    });
    let runs = otlp::import(&export).unwrap();
    assert_eq!(runs[0].calls[1].decisions.len(), 1);
    assert!(runs[0].calls[1].decisions[0]
        .evidence
        .field
        .starts_with("llm.output_messages"));
}
