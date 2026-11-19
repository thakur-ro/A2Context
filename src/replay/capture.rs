//! Inventory the recorded input and locate exact matches in captured requests.

use crate::{Availability, Call, ContextItem, Evidence, Run, TextRange};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct CallCapture<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_evidence: Option<&'a Evidence>,
    pub items: Vec<ItemCapture<'a>>,
}

#[derive(Debug, Serialize)]
pub struct ItemCapture<'a> {
    pub context_id: &'a str,
    pub capture: CaptureAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_fields: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureAvailability {
    Available,
    Redacted,
    ReferenceOnly,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct ContextFieldMatch<'a> {
    pub context_id: &'a str,
    pub fields: Vec<RequestField>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestField {
    pub pointer: String,
    pub kind: RequestFieldKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_range: Option<TextRange>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestFieldKind {
    Text,
    Array,
    Object,
}

pub(crate) fn capture_profiles<'a>(
    runs: impl Iterator<Item = &'a Run>,
) -> Result<Vec<CallCapture<'a>>, String> {
    let mut profiles = Vec::new();
    for run in runs {
        for call in &run.calls {
            if !has_input_profile(call) {
                continue;
            }
            let mut items: Vec<_> = call
                .context
                .iter()
                .map(|item| ItemCapture {
                    context_id: &item.id,
                    capture: capture_availability(item),
                    matching_fields: None,
                })
                .collect();
            if let Some(request) = &call.recorded_request {
                let request = request.parse()?;
                for (item, fields) in items.iter_mut().zip(match_fields(call, &request)?) {
                    if item.capture == CaptureAvailability::Available {
                        item.matching_fields = Some(fields.len());
                    }
                }
            }
            profiles.push(CallCapture {
                run_id: &run.id,
                call_id: &call.id,
                request_evidence: call
                    .recorded_request
                    .as_ref()
                    .map(|request| &request.evidence),
                items,
            });
        }
    }
    Ok(profiles)
}

pub(crate) fn has_input_profile(call: &Call) -> bool {
    call.recorded_request.is_some()
        || (call.retrieval.is_none()
            && (call.model.is_some()
                || !call.context.is_empty()
                || call
                    .decisions
                    .iter()
                    .any(|decision| decision.kind != "tool_execution")))
}

pub(crate) fn capture_availability(item: &ContextItem) -> CaptureAvailability {
    if item.availability == Availability::Included && item.text.is_some() {
        CaptureAvailability::Available
    } else if item.availability == Availability::Redacted {
        CaptureAvailability::Redacted
    } else if item.content_reference.is_some() || item.evidence.field == "gen_ai.input.messages_ref"
    {
        CaptureAvailability::ReferenceOnly
    } else {
        CaptureAvailability::Unavailable
    }
}

pub(super) fn captured_container(text: &str) -> Option<Value> {
    serde_json::from_str(text)
        .ok()
        .filter(|value: &Value| value.is_object() || value.is_array())
}

pub fn request_field_matches<'a>(
    call: &'a Call,
    request: &Value,
) -> Result<Vec<ContextFieldMatch<'a>>, String> {
    Ok(call
        .context
        .iter()
        .zip(match_fields(call, request)?)
        .filter(|(_, fields)| !fields.is_empty())
        .map(|(item, fields)| ContextFieldMatch {
            context_id: &item.id,
            fields,
        })
        .collect())
}

fn match_fields(call: &Call, request: &Value) -> Result<Vec<Vec<RequestField>>, String> {
    let root = request
        .as_object()
        .ok_or("Choose a JSON object containing the inference request.")?;
    let candidates: Vec<_> = call
        .context
        .iter()
        .map(|item| {
            if item.request_fragment.is_some() {
                return None;
            }
            let text = item
                .text
                .as_deref()
                .filter(|_| item.availability == Availability::Included)?;
            Some((text, captured_container(text)))
        })
        .collect();
    let mut text_paths: HashMap<_, Vec<_>> = candidates
        .iter()
        .flatten()
        .map(|(text, _)| (*text, Vec::new()))
        .collect();
    let mut container_paths: HashMap<_, Vec<_>> = candidates
        .iter()
        .flatten()
        .filter_map(|(_, container)| Some((container.as_ref()?, Vec::new())))
        .collect();

    // Edits target children of the request. The root object is not a replay candidate.
    let mut pending: Vec<_> = if text_paths.is_empty() {
        Vec::new()
    } else {
        root.iter()
            .rev()
            .map(|(key, value)| (child_path("", key), value))
            .collect()
    };
    while let Some((pointer, value)) = pending.pop() {
        let kind = match value {
            Value::String(text) => {
                if let Some(paths) = text_paths.get_mut(text.as_str()) {
                    paths.push(RequestField {
                        pointer,
                        kind: RequestFieldKind::Text,
                        text_range: None,
                    });
                }
                continue;
            }
            Value::Object(object) => {
                pending.extend(
                    object
                        .iter()
                        .rev()
                        .map(|(key, value)| (child_path(&pointer, key), value)),
                );
                RequestFieldKind::Object
            }
            Value::Array(array) => {
                pending.extend(
                    array
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(index, value)| (format!("{pointer}/{index}"), value)),
                );
                RequestFieldKind::Array
            }
            _ => continue,
        };
        if !container_paths.is_empty() {
            if let Some(paths) = container_paths.get_mut(value) {
                paths.push(RequestField {
                    pointer,
                    kind,
                    text_range: None,
                });
            }
        }
    }
    Ok(call
        .context
        .iter()
        .zip(&candidates)
        .map(|(item, candidate)| {
            if let Some(fragment) = &item.request_fragment {
                if fragment.validate().is_err() {
                    return Vec::new();
                }
                let captured = item
                    .text
                    .as_deref()
                    .filter(|_| item.availability == Availability::Included);
                let selected = fragment.text(request);
                return if selected.is_some() && selected == captured {
                    vec![RequestField {
                        pointer: fragment.pointer.clone(),
                        kind: RequestFieldKind::Text,
                        text_range: Some(fragment.text_range),
                    }]
                } else {
                    Vec::new()
                };
            }
            let Some((text, container)) = candidate else {
                return Vec::new();
            };
            let mut fields = text_paths[text].clone();
            if let Some(container) = container {
                fields.extend(container_paths[container].iter().cloned());
            }
            fields
        })
        .collect())
}

fn child_path(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}
