//! Normalize GenAI request and response attributes with span-local evidence anchors.

use crate::{Availability, Call, ContextItem, Decision, Evidence, ToolResult};
use serde_json::Value;

pub(crate) fn empty_call(
    span_id: &str,
    trace_id: Option<String>,
    started_at: Option<String>,
) -> Call {
    Call {
        id: span_id.to_owned(),
        trace_id,
        span_id: Some(span_id.to_owned()),
        turn_id: None,
        started_at,
        model: None,
        ended_at: None,
        retrieval: None,
        recorded_request: None,
        context_metadata: None,
        context: Vec::new(),
        decisions: Vec::new(),
        tool_results: Vec::new(),
        relationships: Vec::new(),
    }
}

pub(crate) fn record_inference(mut call: Call, attrs: &Value) -> Result<Call, String> {
    let source_span = call.span_id.clone();
    let anchor = |field: &str, message_index, part_index| Evidence {
        span_id: source_span.clone(),
        field: field.to_owned(),
        message_index,
        part_index,
    };
    call.model = optional_str(attrs, "gen_ai.request.model");

    let input_field = "gen_ai.input.messages";
    let inputs = array_field(attrs, input_field)?;
    for (message_index, message) in inputs.iter().enumerate() {
        let role = optional_str(message, "role");
        for (part_index, part) in array_field(message, "parts")?.iter().enumerate() {
            let evidence = anchor(input_field, Some(message_index), Some(part_index));
            let kind = optional_str(part, "type").unwrap_or_else(|| "message_part".into());
            if kind == "tool_call_response" {
                if let Some(id) = optional_str(part, "id") {
                    call.tool_results.push(ToolResult {
                        tool_call_id: id,
                        outcome: part
                            .get("response")
                            .or_else(|| part.get("result"))
                            .or_else(|| part.get("content"))
                            .cloned(),
                        evidence: evidence.clone(),
                    });
                }
            }
            let text = part
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| crate::canonical_json(part));
            call.context.push(context_item(
                format!("input:{message_index}:{part_index}"),
                kind,
                role.clone(),
                text,
                evidence,
            ));
        }
    }
    for (field, kind) in [
        ("gen_ai.system_instructions", "system_instruction"),
        ("gen_ai.tool.definitions", "tool_definition"),
    ] {
        for (index, value) in array_field(attrs, field)?.into_iter().enumerate() {
            let text = value
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| crate::canonical_json(&value));
            call.context.push(context_item(
                format!("{kind}:{index}"),
                kind.into(),
                None,
                text,
                anchor(field, None, Some(index)),
            ));
        }
    }

    let output_field = "gen_ai.output.messages";
    for (message_index, message) in array_field(attrs, output_field)?.iter().enumerate() {
        for (part_index, part) in array_field(message, "parts")?.iter().enumerate() {
            let (kind, label, outcome, tool_call_id) =
                match part.get("type").and_then(Value::as_str) {
                    Some("tool_call") => (
                        "tool_call",
                        optional_str(part, "name").unwrap_or_else(|| "Tool call".into()),
                        part.get("arguments").cloned().unwrap_or(Value::Null),
                        optional_str(part, "id"),
                    ),
                    Some("text") => {
                        let Some(text) = part.get("content").and_then(Value::as_str) else {
                            continue;
                        };
                        if text.trim().is_empty() {
                            continue;
                        }
                        (
                            "model_output",
                            "Model answer".into(),
                            Value::String(text.to_owned()),
                            None,
                        )
                    }
                    _ => continue,
                };
            call.decisions.push(Decision {
                id: format!("output:{message_index}:{part_index}"),
                kind: kind.into(),
                label,
                outcome,
                evidence: anchor(output_field, Some(message_index), Some(part_index)),
                tool_call_id,
                applies_to: None,
            });
        }
    }
    Ok(call)
}

pub(crate) fn context_item(
    id: String,
    kind: String,
    role: Option<String>,
    text: String,
    evidence: Evidence,
) -> ContextItem {
    ContextItem {
        id,
        kind,
        role,
        text: Some(text),
        content_reference: None,
        document_identity: None,
        request_fragment: None,
        source_id: None,
        source_version: None,
        authority: None,
        availability: Availability::Included,
        evidence,
        rule: None,
    }
}

pub(crate) fn decoded(value: Option<&Value>, field: &str) -> Result<Value, String> {
    match value {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).map_err(|error| format!("invalid JSON in {field}: {error}"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

fn array_field(object: &Value, field: &str) -> Result<Vec<Value>, String> {
    match decoded(object.get(field), field)? {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => Ok(items),
        _ => Err(format!("{field} must be an array")),
    }
}

pub(crate) fn optional_str(object: &Value, field: &str) -> Option<String> {
    object.get(field).and_then(Value::as_str).map(str::to_owned)
}
