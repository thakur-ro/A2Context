//! Exact links between recorded decisions and context already present on the call.

mod outcomes;

use crate::{Availability, Call, ContextItem, Decision};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct ContextLink<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub decision_id: &'a str,
    pub context_id: &'a str,
    pub kind: LinkKind,
    pub argument_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_path: Option<String>,
    pub context_path: Option<String>,
    pub excerpt: String,
    pub source_tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_tool: Option<ToolOrigin<'a>>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ToolOrigin<'a> {
    pub call_id: &'a str,
    pub decision_id: &'a str,
    pub tool: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    ToolDefinition,
    ArgumentMatch,
    OutcomeMatch,
    OutcomeIdentifier,
}

pub(crate) fn result_invocation<'a>(call: &'a Call, item: &ContextItem) -> Option<&'a str> {
    let mut result_ids = call
        .tool_results
        .iter()
        .filter(|result| result.evidence == item.evidence)
        .map(|result| result.tool_call_id.as_str());
    result_ids
        .next()
        .filter(|id| result_ids.all(|other| other == *id))
}

pub(crate) fn tool_definition_name<'a>(
    item: &'a ContextItem,
    text: &'a str,
) -> Option<(Cow<'a, str>, Option<&'static str>)> {
    if item.evidence.field.starts_with("llm.tools.") && item.evidence.field.ends_with(".tool.name")
    {
        return Some((Cow::Borrowed(text), None));
    }
    let definition = serde_json::from_str::<Value>(text).ok()?;
    let (name, path) = definition
        .get("name")
        .and_then(Value::as_str)
        .map(|name| (name, "/name"))
        .or_else(|| {
            definition
                .pointer("/function/name")
                .and_then(Value::as_str)
                .map(|name| (name, "/function/name"))
        })?;
    Some((Cow::Owned(name.to_owned()), Some(path)))
}

pub(crate) fn link_call<'a>(
    run_id: &'a str,
    call: &'a Call,
    earlier_tools: &BTreeMap<&'a str, Option<ToolOrigin<'a>>>,
) -> Vec<ContextLink<'a>> {
    let mut links = link_tools(run_id, call, earlier_tools);
    outcomes::link_outcomes(run_id, call, earlier_tools, &mut links);
    links
}

fn link_tools<'a>(
    run_id: &'a str,
    call: &'a Call,
    earlier_tools: &BTreeMap<&'a str, Option<ToolOrigin<'a>>>,
) -> Vec<ContextLink<'a>> {
    let decisions: Vec<_> = call
        .decisions
        .iter()
        .filter(|decision| decision.kind == "tool_call")
        .collect();
    if decisions.is_empty() {
        return Vec::new();
    }

    let mut links = Vec::new();
    for item in call
        .context
        .iter()
        .filter(|item| item.availability == Availability::Included)
    {
        let Some(text) = item.text.as_deref() else {
            continue;
        };
        let source_tool_call_id = result_invocation(call, item);
        let source_tool =
            source_tool_call_id.and_then(|id| earlier_tools.get(id).copied().flatten());
        let mut push = |decision: &'a Decision, kind, argument_path, context_path, excerpt| {
            links.push(ContextLink {
                run_id,
                call_id: &call.id,
                decision_id: &decision.id,
                context_id: &item.id,
                kind,
                argument_path,
                outcome_path: None,
                context_path,
                excerpt,
                source_tool_call_id,
                source_tool,
            });
        };
        match item.kind.as_str() {
            "tool_definition" => {
                if let Some((name, path)) = tool_definition_name(item, text) {
                    for decision in decisions
                        .iter()
                        .filter(|decision| decision.label == name.as_ref())
                    {
                        push(
                            decision,
                            LinkKind::ToolDefinition,
                            None,
                            path.map(str::to_owned),
                            name.to_string(),
                        );
                    }
                }
            }
            "tool_call_response" | "tool_result" => {
                let body = result_body(item.kind.as_str(), text);
                let mut values = Vec::new();
                collect_result_values(&body, "", None, &mut values);
                for decision in &decisions {
                    match_arguments(decision, |key, path, value| {
                        if let Some(matched) = values.iter().find(|source| {
                            source.value == *value
                                && (source.key.as_deref() == Some(key)
                                    || (source.key.is_none() && distinctive_string(value)))
                        }) {
                            push(
                                decision,
                                LinkKind::ArgumentMatch,
                                Some(path.to_owned()),
                                Some(matched.path.clone()),
                                display_value(value),
                            );
                        }
                    });
                }
                // A simulated user message can be delivered as a structured tool result.
                if let Some(message) = user_message(&body) {
                    for decision in &decisions {
                        match_arguments(decision, |_, path, value| {
                            if let Some(value) = value.as_str().filter(|value| identifier(value)) {
                                if exact_token(&message, value)
                                    && !values.iter().any(|source| source.value == value)
                                {
                                    push(
                                        decision,
                                        LinkKind::ArgumentMatch,
                                        Some(path.to_owned()),
                                        None,
                                        value.to_owned(),
                                    );
                                }
                            }
                        });
                    }
                }
            }
            "message" | "message_part" | "text" if item.role.as_deref() == Some("user") => {
                for decision in &decisions {
                    match_arguments(decision, |_, path, value| {
                        if let Some(value) = value.as_str().filter(|value| identifier(value)) {
                            if exact_token(text, value) {
                                push(
                                    decision,
                                    LinkKind::ArgumentMatch,
                                    Some(path.to_owned()),
                                    None,
                                    value.to_owned(),
                                );
                            }
                        }
                    });
                }
            }
            _ => {}
        }
    }
    links
}

fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| crate::canonical_json(value))
}

fn match_arguments(decision: &Decision, mut matching: impl FnMut(&str, &str, &Value)) {
    let parsed;
    let args = if let Some(raw) = decision.outcome.as_str() {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        parsed = value;
        &parsed
    } else {
        &decision.outcome
    };
    visit_values(args, "", None, &mut |value, path, key| {
        if let Some(key) = key {
            if value.is_number() || value.as_str().is_some_and(|text| !text.trim().is_empty()) {
                matching(key, path, value);
            }
        }
    });
}

fn result_body(kind: &str, text: &str) -> Value {
    let decoded =
        serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.to_owned()));
    if kind == "tool_call_response" {
        return decoded
            .get("result")
            .or_else(|| decoded.get("response"))
            .or_else(|| decoded.get("content"))
            .cloned()
            .unwrap_or(Value::Null);
    }
    decoded
}

struct ResultValue {
    value: Value,
    path: String,
    key: Option<String>,
}

fn collect_result_values(
    value: &Value,
    path: &str,
    key: Option<&str>,
    values: &mut Vec<ResultValue>,
) {
    // MCP content blocks contain a text payload that can itself hold JSON.
    if let Some(items) = value.as_array() {
        for (index, item) in items.iter().enumerate() {
            let location = format!("{path}/{index}");
            if item.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let payload = serde_json::from_str::<Value>(text)
                        .ok()
                        .filter(|value| value.is_object() || value.is_array())
                        .unwrap_or_else(|| Value::String(text.to_owned()));
                    save_result_values(&payload, &format!("{location}/text"), key, values);
                    continue;
                }
            }
            collect_result_values(item, &location, key, values);
        }
    } else {
        save_result_values(value, path, key, values);
    }
}

fn save_result_values(value: &Value, path: &str, key: Option<&str>, values: &mut Vec<ResultValue>) {
    visit_values(value, path, key, &mut |value, position, field| {
        if value.is_number() || value.as_str().is_some_and(|text| !text.trim().is_empty()) {
            values.push(ResultValue {
                value: value.clone(),
                path: position.into(),
                key: field.map(str::to_owned),
            });
        }
    });
}

fn visit_values(
    value: &Value,
    path: &str,
    key: Option<&str>,
    visit: &mut impl FnMut(&Value, &str, Option<&str>),
) {
    match value {
        Value::Object(fields) => {
            for (field, value) in fields {
                let encoded = field.replace('~', "~0").replace('/', "~1");
                visit_values(value, &format!("{path}/{encoded}"), Some(field), visit);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                visit_values(item, &format!("{path}/{index}"), key, visit);
            }
        }
        _ => visit(value, path, key),
    }
}

fn user_message(value: &Value) -> Option<String> {
    fn message(value: &Value) -> Option<String> {
        (value.get("sender").and_then(Value::as_str) == Some("user"))
            .then(|| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten()
    }
    message(value).or_else(|| {
        value.as_array()?.iter().find_map(|item| {
            if item["type"] != "text" {
                return None;
            }
            let payload = serde_json::from_str::<Value>(item["text"].as_str()?).ok()?;
            message(&payload)
        })
    })
}

fn distinctive_string(value: &Value) -> bool {
    value.as_str().is_some_and(identifier)
}

fn identifier(value: &str) -> bool {
    value.len() >= 6
        && (value.contains('@') || value.bytes().any(|byte| byte.is_ascii_digit()))
        && value
            .chars()
            .all(|character| token_char(character) || character == '.')
}

fn token_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '@' | '_' | '-' | '+' | '#' | '/')
}

fn exact_token(text: &str, value: &str) -> bool {
    text.match_indices(value).any(|(start, _)| {
        let before = &text[..start];
        let after = &text[start + value.len()..];
        !before.chars().next_back().is_some_and(token_char) && !after.chars().next().is_some_and(token_char)
            // A dot within a token counts as an identifier character, while sentence punctuation does not.
            && !(before.ends_with('.') && before[..before.len() - 1].chars().next_back().is_some_and(token_char))
            && !(after.starts_with('.') && after[1..].chars().next().is_some_and(token_char))
    })
}
