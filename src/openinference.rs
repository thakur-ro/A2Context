//! Normalize OpenInference attributes while retaining the original flattened paths.

use crate::{
    genai::context_item, Availability, Call, ContextItem, Decision, DocumentIdentity, Evidence,
    Retrieval, ToolResult,
};
use serde_json::Value;
use std::collections::BTreeMap;

const KIND: &str = "openinference.span.kind";
const REDACTED: &str = "__REDACTED__";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Llm,
    Tool,
    Decision,
    Guardrail,
    Retriever,
}

pub(crate) fn kind(attrs: &Value) -> Option<Kind> {
    match attrs[KIND].as_str()? {
        "LLM" => Some(Kind::Llm),
        "TOOL" => Some(Kind::Tool),
        "DECISION" => Some(Kind::Decision),
        "GUARDRAIL" => Some(Kind::Guardrail),
        "RETRIEVER" => Some(Kind::Retriever),
        _ => None,
    }
}

pub(crate) fn scope_id(attrs: &Value, field: &str) -> Option<String> {
    attrs.get(KIND)?;
    available(attrs.get(field)?)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

#[derive(Clone, Copy)]
struct Field<'a> {
    path: &'a str,
    value: &'a Value,
}

type Fields<'a> = BTreeMap<&'a str, Field<'a>>;

pub(crate) fn record(call: &mut Call, attrs: &Value, name: Option<&str>, kind: Kind) {
    let Some(attrs) = attrs.as_object() else {
        return;
    };
    let fields: Fields = attrs
        .iter()
        .map(|(path, value)| (path.as_str(), Field { path, value }))
        .collect();
    match kind {
        Kind::Llm => record_llm(call, &fields),
        Kind::Retriever => {
            if let Some(field) = fields.get("input.value") {
                let query = payload(&fields, "input.value", "input.mime_type");
                add_context(
                    call,
                    field.path,
                    "retrieval_query",
                    None,
                    query.as_ref().unwrap_or(field.value),
                );
            }
            call.retrieval = Some(Retrieval {
                label: name.unwrap_or("Retrieval").into(),
                documents: indexed(&fields, "retrieval.documents.")
                    .values()
                    .filter_map(|fields| document(call, fields, "", "retrieved_document", None))
                    .collect(),
            });
        }
        Kind::Tool => {
            let output = payload(&fields, "output.value", "output.mime_type");
            let label = string(&fields, "tool.name")
                .or(name)
                .unwrap_or("Tool execution");
            let path = if fields.contains_key("output.value") {
                "output.value"
            } else if fields.contains_key("tool.name") {
                "tool.name"
            } else {
                KIND
            };
            add_decision(call, path, "tool_execution", label, output.clone(), None);
            if let Some(id) = string(&fields, "tool.id") {
                let path = if output.is_some() {
                    "output.value"
                } else {
                    "tool.id"
                };
                call.tool_results.push(ToolResult {
                    tool_call_id: id.into(),
                    outcome: output,
                    evidence: evidence(call, path),
                });
            }
        }
        Kind::Decision | Kind::Guardrail => {
            let (kind, default_label) = if kind == Kind::Decision {
                ("decision_model", "Decision model")
            } else {
                ("guardrail", "Guardrail")
            };
            call.model = model(&fields, "decision");
            if let Some(field) = fields.get("input.value") {
                let value = payload(&fields, "input.value", "input.mime_type");
                if let Some(Value::Object(object)) = value
                    .as_ref()
                    .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
                {
                    for (key, value) in object {
                        let pointer = key.replace('~', "~0").replace('/', "~1");
                        add_context(
                            call,
                            &format!("input.value#/{pointer}"),
                            "request_field",
                            None,
                            value,
                        );
                    }
                } else {
                    add_context(
                        call,
                        field.path,
                        "request",
                        None,
                        value.as_ref().unwrap_or(field.value),
                    );
                }
            }
            let path = if fields.contains_key("output.value") {
                "output.value"
            } else {
                KIND
            };
            add_decision(
                call,
                path,
                kind,
                name.unwrap_or(default_label),
                payload(&fields, "output.value", "output.mime_type"),
                None,
            );
        }
    }
}

fn record_llm(call: &mut Call, fields: &Fields<'_>) {
    call.model = model(fields, "llm");
    let inputs = indexed(fields, "llm.input_messages.");
    if inputs.is_empty() {
        if let Some(field) = fields.get("input.value") {
            let value = payload(fields, "input.value", "input.mime_type");
            add_context(
                call,
                field.path,
                "request",
                None,
                value.as_ref().unwrap_or(field.value),
            );
        }
    }
    for message in inputs.values() {
        let role = string(message, "message.role");
        let is_tool = matches!(role, Some("tool" | "function"));
        for part in parts(message) {
            match part {
                Part::Content(field) => {
                    let kind = if is_tool {
                        "tool_call_response"
                    } else {
                        "message"
                    };
                    add_context(call, field.path, kind, role, field.value);
                    if let Some(id) = string(message, "message.tool_call_id").filter(|_| is_tool) {
                        call.tool_results.push(ToolResult {
                            tool_call_id: id.into(),
                            outcome: available(field.value).map(json_container),
                            evidence: evidence(call, field.path),
                        });
                    }
                }
                Part::Tool(tool) => {
                    if let Some(field) = tool.get("tool_call.function.arguments") {
                        add_context(call, field.path, "tool_call_arguments", role, field.value);
                    }
                }
                Part::Document(fields) => {
                    if let Some(item) = document(
                        call,
                        &fields,
                        "message_content.document.",
                        "input_document",
                        role,
                    ) {
                        call.context.push(item);
                    }
                }
            }
        }
    }
    let documents = indexed(fields, "input.documents.");
    let items: Vec<_> = documents
        .values()
        .filter_map(|fields| document(call, fields, "", "input_document", None))
        .collect();
    call.context.extend(items);

    for tool in indexed(fields, "llm.tools.").values() {
        if let Some(field) = tool
            .get("tool.json_schema")
            .or_else(|| tool.get("tool.name"))
        {
            add_context(call, field.path, "tool_definition", None, field.value);
        }
    }

    let outputs = indexed(fields, "llm.output_messages.");
    if outputs.is_empty() && fields.contains_key("output.value") {
        add_decision(
            call,
            "output.value",
            "model_output",
            "Model output",
            payload(fields, "output.value", "output.mime_type"),
            None,
        );
    }
    for message in outputs.values() {
        for part in parts(message) {
            match part {
                Part::Content(field) => {
                    if !field
                        .value
                        .as_str()
                        .is_some_and(|text| text.trim().is_empty())
                    {
                        add_decision(
                            call,
                            field.path,
                            "model_output",
                            "Model answer",
                            available(field.value).cloned(),
                            None,
                        );
                    }
                }
                Part::Tool(tool) => {
                    let args = tool.get("tool_call.function.arguments");
                    let name = tool.get("tool_call.function.name");
                    let id = tool.get("tool_call.id");
                    if let Some(field) = args.or(name).or(id) {
                        add_decision(
                            call,
                            field.path,
                            "tool_call",
                            string(&tool, "tool_call.function.name").unwrap_or("Tool call"),
                            args.and_then(|field| available(field.value))
                                .map(json_payload),
                            string(&tool, "tool_call.id"),
                        );
                    }
                }
                Part::Document(fields) => {
                    if let Some(field) = fields.get("message_content.document.document.content") {
                        add_decision(
                            call,
                            field.path,
                            "model_output",
                            "Model answer",
                            available(field.value).cloned(),
                            None,
                        );
                    }
                }
            }
        }
    }
}

fn indexed<'a>(fields: &Fields<'a>, prefix: &str) -> BTreeMap<usize, Fields<'a>> {
    let mut indexed = BTreeMap::<_, Fields>::new();
    for (&key, &field) in fields {
        let Some((index, suffix)) = key.strip_prefix(prefix).and_then(|key| key.split_once('.'))
        else {
            continue;
        };
        if let Ok(index) = index.parse::<usize>() {
            indexed.entry(index).or_default().insert(suffix, field);
        }
    }
    indexed
}

enum Part<'a> {
    Content(Field<'a>),
    Tool(Fields<'a>),
    Document(Fields<'a>),
}

fn parts<'a>(message: &Fields<'a>) -> Vec<Part<'a>> {
    let mut parts: Vec<_> = indexed(message, "message.contents.")
        .into_values()
        .filter_map(|part| match string(&part, "message_content.type") {
            Some("reasoning") => None,
            Some("tool_use") => Some(Part::Tool(part)),
            Some("document") => Some(Part::Document(part)),
            _ if part
                .keys()
                .any(|key| key.starts_with("message_content.document.")) =>
            {
                Some(Part::Document(part))
            }
            _ => part.get("message_content.text").copied().map(Part::Content),
        })
        .collect();
    if !parts.iter().any(|part| match part {
        Part::Content(_) => true,
        Part::Document(fields) => fields.contains_key("message_content.document.document.content"),
        Part::Tool(_) => false,
    }) {
        if let Some(&field) = message.get("message.content") {
            parts.insert(0, Part::Content(field));
        }
    }
    if !parts.iter().any(|part| matches!(part, Part::Tool(_))) {
        parts.extend(
            indexed(message, "message.tool_calls.")
                .into_values()
                .map(Part::Tool),
        );
    }
    parts
}

fn string<'a>(fields: &Fields<'a>, key: &str) -> Option<&'a str> {
    let text = fields.get(key)?.value.as_str()?;
    (text != REDACTED && !text.trim().is_empty()).then_some(text)
}

fn model(fields: &Fields<'_>, namespace: &str) -> Option<String> {
    ["response.model_name", "model_name", "request.model_name"]
        .into_iter()
        .find_map(|key| string(fields, &format!("{namespace}.{key}")).map(str::to_owned))
}

fn available(value: &Value) -> Option<&Value> {
    (!value.is_null() && value.as_str() != Some(REDACTED)).then_some(value)
}

fn json_payload(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| value.clone())
}

fn json_container(value: &Value) -> Value {
    let parsed = json_payload(value);
    if parsed.is_object() || parsed.is_array() {
        parsed
    } else {
        value.clone()
    }
}

fn payload(fields: &Fields<'_>, field: &str, mime: &str) -> Option<Value> {
    let value = available(fields.get(field)?.value)?;
    let mime = string(fields, mime).and_then(|text| text.split(';').next());
    Some(match mime.map(str::trim) {
        Some(media_type) if media_type == "application/json" || media_type.ends_with("+json") => {
            json_payload(value)
        }
        Some(_) => value.clone(),
        None => json_container(value),
    })
}

fn evidence(call: &Call, path: &str) -> Evidence {
    Evidence {
        span_id: call.span_id.clone(),
        field: path.into(),
        message_index: None,
        part_index: None,
    }
}

fn add_context(call: &mut Call, path: &str, kind: &str, role: Option<&str>, value: &Value) {
    call.context
        .push(capture_context(call, path, kind, role, value));
}

fn capture_context(
    call: &Call,
    path: &str,
    kind: &str,
    role: Option<&str>,
    value: &Value,
) -> ContextItem {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| crate::canonical_json(value));
    let mut item = context_item(
        path.into(),
        kind.into(),
        role.map(str::to_owned),
        text,
        evidence(call, path),
    );
    if available(value).is_none() {
        item.text = None;
        item.availability = if value.is_null() {
            Availability::Unknown
        } else {
            Availability::Redacted
        };
    }
    item
}

fn document(
    call: &Call,
    fields: &Fields<'_>,
    prefix: &str,
    kind: &str,
    role: Option<&str>,
) -> Option<ContextItem> {
    let content = fields.get(format!("{prefix}document.content").as_str());
    let id = fields.get(format!("{prefix}document.id").as_str());
    let url = fields.get(format!("{prefix}document.url").as_str());
    let path = content.or(id).or(url)?.path;
    let mut item = capture_context(
        call,
        path,
        kind,
        role,
        content.map_or(&Value::Null, |field| field.value),
    );
    item.document_identity = id.and_then(|field| {
        let value = available(field.value)?;
        let id = match value {
            Value::String(value) if !value.trim().is_empty() => value.clone(),
            Value::Number(value) if value.is_i64() || value.is_u64() => value.to_string(),
            _ => return None,
        };
        Some(DocumentIdentity {
            id,
            evidence: evidence(call, field.path),
        })
    });
    item.content_reference = url
        .and_then(|field| available(field.value)?.as_str())
        .filter(|value| !value.is_empty() && !value.starts_with("data:"))
        .map(str::to_owned);
    Some(item)
}

fn add_decision(
    call: &mut Call,
    path: &str,
    kind: &str,
    label: &str,
    outcome: Option<Value>,
    tool_id: Option<&str>,
) {
    call.decisions.push(Decision {
        id: path.into(),
        kind: kind.into(),
        label: label.into(),
        outcome: outcome.unwrap_or(Value::Null),
        evidence: evidence(call, path),
        tool_call_id: tool_id.map(str::to_owned),
        applies_to: None,
    });
}
