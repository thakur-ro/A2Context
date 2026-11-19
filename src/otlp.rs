//! Normalize captured OTLP spans and merge batches by trace and span ID.

use crate::genai::{empty_call, optional_str, record_inference};
use crate::openinference::{self, Kind};
use crate::{
    Availability, Call, ContextItem, Decision, Evidence, Run, ToolResult, TraceSpan, SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Export {
    #[serde(default)]
    resource_spans: Vec<ResourceSpans>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceSpans {
    #[serde(default)]
    resource: Option<Resource>,
    #[serde(default)]
    scope_spans: Vec<ScopeSpans>,
}

#[derive(Default, Deserialize)]
struct Resource {
    #[serde(default)]
    attributes: Vec<Attribute>,
}

#[derive(Deserialize)]
struct ScopeSpans {
    #[serde(default)]
    spans: Vec<Span>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Span {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    name: Option<String>,
    start_time_unix_nano: Option<Value>,
    end_time_unix_nano: Option<Value>,
    #[serde(default)]
    attributes: Vec<Attribute>,
}

#[derive(Deserialize)]
struct Attribute {
    key: String,
    #[serde(default)]
    value: Value,
}

#[derive(Clone, Debug, PartialEq)]
struct RecordedSpan {
    /// Nanoseconds are kept for ordering before formatting timestamps for the inspector.
    start: Option<i128>,
    end: Option<i128>,
    parent_span_id: Option<String>,
    name: Option<String>,
    attrs: Value,
    organization_id: Option<String>,
    user_id: Option<String>,
    conversation_id: Option<String>,
}

type Spans = BTreeMap<String, (usize, RecordedSpan)>;

#[derive(Default)]
pub struct Archive {
    traces: BTreeMap<String, Trace>,
}

struct Trace {
    spans: Spans,
    run: Option<Run>,
}

/// The updates in a batch are validated together before any are applied.
pub struct Batch {
    traces: BTreeMap<String, Trace>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.traces.is_empty()
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.traces.values().filter_map(|trace| trace.run.as_ref())
    }
}

/// Accept one export request or an array of export requests (including decoded JSONL).
pub fn import(document: &Value) -> Result<Vec<Run>, String> {
    let archive = Archive::default();
    let batch = archive.prepare(document)?;
    let runs: Vec<_> = batch
        .traces
        .into_values()
        .filter_map(|trace| trace.run)
        .collect();
    if runs.is_empty() {
        return Err("the OTLP export contains no supported GenAI or OpenInference spans".into());
    }
    Ok(runs)
}

impl Archive {
    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.traces.values().filter_map(|trace| trace.run.as_ref())
    }

    pub fn commit(&mut self, batch: Batch) {
        self.traces.extend(batch.traces);
    }

    pub fn prepare(&self, document: &Value) -> Result<Batch, String> {
        let exports = document
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(std::slice::from_ref(document));
        let mut updates = BTreeMap::<String, Spans>::new();
        for (index, value) in exports.iter().enumerate() {
            let export: Export = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid OTLP export {}: {error}", index + 1))?;
            for resource in export.resource_spans {
                let resource_attrs = decode_attrs(
                    resource
                        .resource
                        .as_ref()
                        .map(|resource| resource.attributes.as_slice())
                        .unwrap_or_default(),
                )?;
                for scope in resource.scope_spans {
                    for span in scope.spans {
                        let trace_id = normalize_id(&span.trace_id, 32, "traceId")?;
                        let span_id = normalize_id(&span.span_id, 16, "spanId")?;
                        let attrs = decode_attrs(&span.attributes)?;
                        let record = RecordedSpan {
                            start: parse_time(span.start_time_unix_nano.as_ref())
                                .map_err(|error| format!("span {span_id}: {error}"))?,
                            end: parse_time(span.end_time_unix_nano.as_ref())
                                .map_err(|error| format!("span {span_id}: {error}"))?
                                .filter(|end| *end != 0),
                            name: span.name.filter(|name| !name.is_empty()),
                            parent_span_id: span
                                .parent_span_id
                                .filter(|id| !id.is_empty())
                                .map(|id| id.to_ascii_lowercase()),
                            organization_id: identity(
                                &attrs,
                                &resource_attrs,
                                "nexusweave.organization.id",
                            ),
                            user_id: identity(&attrs, &resource_attrs, "nexusweave.user.id")
                                .or_else(|| openinference::scope_id(&attrs, "user.id")),
                            conversation_id: optional_str(&attrs, "gen_ai.conversation.id")
                                .or_else(|| openinference::scope_id(&attrs, "session.id")),
                            attrs,
                        };
                        let known = self.traces.get(&trace_id).map(|trace| &trace.spans);
                        let spans = updates.get(&trace_id).or(known);
                        if let Some((_, existing)) = spans.and_then(|spans| spans.get(&span_id)) {
                            if existing != &record {
                                return Err(format!("conflicting copies of OTLP span {span_id}"));
                            }
                        } else {
                            let spans = updates
                                .entry(trace_id)
                                .or_insert_with(|| known.cloned().unwrap_or_default());
                            spans.insert(span_id, (spans.len(), record));
                        }
                    }
                }
            }
        }

        let traces = updates
            .into_iter()
            .map(|(trace_id, spans)| {
                let run = normalize_trace(&trace_id, &spans)?;
                Ok((trace_id, Trace { spans, run }))
            })
            .collect::<Result<_, String>>()?;
        Ok(Batch { traces })
    }
}

fn normalize_trace(trace_id: &str, spans: &Spans) -> Result<Option<Run>, String> {
    let mut ordered: Vec<_> = spans.iter().collect();
    ordered.sort_by_key(|(_, (position, span))| (span.start.is_none(), span.start, *position));
    let mut calls = Vec::new();
    for (id, (_, span)) in &ordered {
        if let Some(call) = normalize_span(trace_id, id, span)? {
            calls.push(call);
        }
    }
    if calls.is_empty() {
        return Ok(None);
    }
    let trace_spans = if ordered
        .iter()
        .any(|(_, (_, span))| span.parent_span_id.is_some())
    {
        ordered
            .into_iter()
            .map(|(id, (_, span))| {
                Ok(TraceSpan {
                    id: id.clone(),
                    parent_span_id: span.parent_span_id.clone(),
                    name: span.name.clone(),
                    started_at: span.start.map(format_time).transpose()?,
                })
            })
            .collect::<Result<_, String>>()?
    } else {
        Vec::new()
    };
    let run = Run {
        schema_version: SCHEMA_VERSION,
        id: format!("otlp:{trace_id}"),
        source: "otlp".to_owned(),
        organization_id: agreeing(spans, trace_id, "organization", |span| {
            &span.organization_id
        })?,
        user_id: agreeing(spans, trace_id, "user", |span| &span.user_id)?,
        conversation_id: agreeing(spans, trace_id, "conversation", |span| {
            &span.conversation_id
        })?,
        calls,
        spans: trace_spans,
    };
    crate::validate(&run)?;
    Ok(Some(run))
}

fn normalize_span(trace_id: &str, id: &str, span: &RecordedSpan) -> Result<Option<Call>, String> {
    let attrs = &span.attrs;
    let operation = attrs["gen_ai.operation.name"].as_str();
    let explicit_inference = matches!(
        operation,
        Some("chat" | "generate_content" | "text_completion")
    );
    let explicit_tool = operation == Some("execute_tool");
    let use_openinference = openinference::kind(attrs).filter(|kind| {
        if explicit_inference {
            return *kind == Kind::Llm
                && ![
                    "gen_ai.input.messages",
                    "gen_ai.input.messages_ref",
                    "gen_ai.output.messages",
                    "gen_ai.system_instructions",
                    "gen_ai.tool.definitions",
                ]
                .iter()
                .any(|field| attrs.get(field).is_some());
        }
        if explicit_tool {
            return *kind == Kind::Tool
                && attrs.get("gen_ai.tool.call.id").is_none()
                && attrs.get("gen_ai.tool.call.result").is_none();
        }
        true
    });
    let is_inference =
        explicit_inference || (operation.is_none() && attrs.get("gen_ai.request.model").is_some());
    if !is_inference && !explicit_tool && use_openinference.is_none() {
        return Ok(None);
    }

    let time = span.start.map(format_time).transpose()?;
    let mut call = empty_call(id, Some(trace_id.to_owned()), time);
    call.ended_at = span.end.map(format_time).transpose()?;
    call.turn_id = optional_str(attrs, "nexusweave.turn.id");
    if let Some(kind) = use_openinference {
        openinference::record(&mut call, attrs, span.name.as_deref(), kind);
        if kind != Kind::Tool {
            crate::span_metadata::apply(&mut call, attrs, str::to_owned);
        }
        Ok(Some(call))
    } else if is_inference {
        let mut call =
            record_inference(call, attrs).map_err(|error| format!("span {id}: {error}"))?;
        if !call
            .context
            .iter()
            .any(|item| item.evidence.field == "gen_ai.input.messages")
        {
            if let Some(reference) = optional_str(attrs, "gen_ai.input.messages_ref") {
                call.context.push(ContextItem {
                    id: "input-reference".into(),
                    kind: "external_context".into(),
                    role: None,
                    text: None,
                    content_reference: None,
                    document_identity: None,
                    request_fragment: None,
                    source_id: Some(reference),
                    source_version: None,
                    authority: None,
                    availability: Availability::Unknown,
                    evidence: anchor(id, "gen_ai.input.messages_ref"),
                    rule: None,
                });
            }
        }
        crate::span_metadata::apply(&mut call, attrs, str::to_owned);
        Ok(Some(call))
    } else {
        let result_field = "gen_ai.tool.call.result";
        let outcome = attrs
            .get(result_field)
            .filter(|value| !value.is_null())
            .cloned();
        let tool_name = "gen_ai.tool.name";
        let tool_label = optional_str(attrs, tool_name);
        let label_field = if tool_label.is_some() {
            tool_name
        } else {
            "name"
        };
        let label = tool_label
            .or_else(|| span.name.clone())
            .unwrap_or_else(|| "Tool execution".into());
        if outcome.is_some() || attrs.get(tool_name).is_some() || span.name.is_some() {
            call.decisions.push(Decision {
                id: "tool_result".into(),
                kind: "tool_execution".into(),
                label,
                outcome: outcome.clone().unwrap_or(Value::Null),
                evidence: anchor(
                    id,
                    if outcome.is_some() {
                        result_field
                    } else {
                        label_field
                    },
                ),
                tool_call_id: None,
                applies_to: None,
            });
        }
        let id_field = "gen_ai.tool.call.id";
        if let Some(tool_call_id) = optional_str(attrs, id_field) {
            let evidence_field = if outcome.is_some() {
                result_field
            } else {
                id_field
            };
            call.tool_results.push(ToolResult {
                tool_call_id,
                outcome,
                evidence: anchor(id, evidence_field),
            });
        }
        Ok(Some(call))
    }
}

fn anchor(id: &str, field: &str) -> Evidence {
    Evidence {
        span_id: Some(id.to_owned()),
        field: field.to_owned(),
        message_index: None,
        part_index: None,
    }
}

fn identity(attrs: &Value, resource: &Value, field: &str) -> Option<String> {
    optional_str(attrs, field).or_else(|| optional_str(resource, field))
}

fn agreeing(
    spans: &BTreeMap<String, (usize, RecordedSpan)>,
    trace: &str,
    label: &str,
    select: impl Fn(&RecordedSpan) -> &Option<String>,
) -> Result<Option<String>, String> {
    let mut recorded = spans.values().filter_map(|(_, span)| select(span).as_ref());
    let first = recorded.next();
    if recorded.any(|value| Some(value) != first) {
        return Err(format!(
            "conflicting {label} identifiers in OTLP trace {trace}"
        ));
    }
    Ok(first.cloned())
}

fn normalize_id(value: &str, length: usize, field: &str) -> Result<String, String> {
    if value.len() != length
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!(
            "invalid OTLP {field}, expected {length} hexadecimal characters"
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn parse_time(value: Option<&Value>) -> Result<Option<i128>, String> {
    value
        .filter(|value| !value.is_null())
        .map(|value| {
            let time = match value {
                Value::String(text) => text.parse::<u64>().ok(),
                value => value.as_u64(),
            };
            time.map(i128::from)
                .ok_or_else(|| "invalid startTimeUnixNano".to_owned())
        })
        .transpose()
}

fn format_time(nanos: i128) -> Result<String, String> {
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|error| error.to_string())?
        .format(&Rfc3339)
        .map_err(|error| error.to_string())
}

fn decode_attrs(attributes: &[Attribute]) -> Result<Value, String> {
    let values = attributes
        .iter()
        .map(|attribute| decode_value(&attribute.value).map(|value| (attribute.key.clone(), value)))
        .collect::<Result<Map<_, _>, _>>()?;
    Ok(Value::Object(values))
}

fn decode_value(value: &Value) -> Result<Value, String> {
    if let Some(array) = value.get("arrayValue") {
        let members = array["values"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        return members
            .iter()
            .map(decode_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if let Some(list) = value.get("kvlistValue") {
        let entries: Vec<Attribute> = serde_json::from_value(
            list["values"]
                .as_array()
                .cloned()
                .map(Value::Array)
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(|error| error.to_string())?;
        return decode_attrs(&entries);
    }
    if let Some(number) = value.get("intValue") {
        return number
            .as_i64()
            .or_else(|| number.as_str()?.parse().ok())
            .map(Value::from)
            .ok_or_else(|| "invalid OTLP intValue".into());
    }
    Ok(["stringValue", "boolValue", "doubleValue", "bytesValue"]
        .into_iter()
        .find_map(|field| value.get(field))
        .cloned()
        .unwrap_or(Value::Null))
}
