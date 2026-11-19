//! Import recorded LangSmith Run objects without contacting the service.

use crate::genai::empty_call;
use crate::{
    Availability, Call, ContextItem, Decision, Evidence, Run, ToolResult, TraceSpan, SCHEMA_VERSION,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn import(document: &Value) -> Result<Vec<Run>, String> {
    let rows = if let Some(rows) = document
        .as_array()
        .or_else(|| document.get("runs").and_then(Value::as_array))
    {
        rows.as_slice()
    } else if document.get("id").is_some() {
        std::slice::from_ref(document)
    } else {
        return Err(
            "expected a LangSmith run, a runs array, or an object with a runs array".into(),
        );
    };
    let mut traces = BTreeMap::<&str, Vec<&Value>>::new();
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = string(row, "id").ok_or("LangSmith run id is missing")?;
        if !ids.insert(id) {
            return Err(format!("duplicate LangSmith run id {id}"));
        }
        let trace = string(row, "trace_id").unwrap_or(id);
        traces.entry(trace).or_default().push(row);
    }

    let mut runs = Vec::new();
    for (trace, mut rows) in traces {
        // dotted_order is hierarchical. Its last segment describes the current span.
        let complete_dotted_order = rows.iter().all(|row| string(row, "dotted_order").is_some());
        rows.sort_by_key(|row| {
            (
                if complete_dotted_order {
                    string(row, "dotted_order").and_then(|value| value.rsplit('.').next())
                } else {
                    string(row, "start_time")
                },
                string(row, "id"),
            )
        });
        let mut calls = Vec::new();
        for row in &rows {
            let mut call = new_call(row, trace);
            match string(row, "run_type") {
                Some("llm") => {
                    import_inference(row, &mut call)?;
                    let metadata = &row["extra"]["metadata"];
                    // Child runs may inherit metadata from their parent.
                    if string(metadata, "nexusweave.span.id") == Some(call.id.as_str()) {
                        crate::span_metadata::apply(&mut call, metadata, |key| {
                            format!("extra.metadata[{key:?}]")
                        });
                    }
                }
                Some("tool") => import_tool_span(row, &mut call),
                _ => continue,
            }
            calls.push(call);
        }
        if calls.is_empty() {
            continue;
        }

        let thread = shared_field(&rows, trace, "thread", |row| {
            let metadata = &row["extra"]["metadata"];
            string(metadata, "thread_id").or_else(|| string(metadata, "session_id"))
        })?;
        let organization_id = shared_field(&rows, trace, "organization", |row| {
            string(&row["extra"]["metadata"], "organization_id")
        })?;
        let user_id = shared_field(&rows, trace, "user", |row| {
            string(&row["extra"]["metadata"], "user_id")
        })?;
        let project = shared_field(&rows, trace, "project", |row| string(row, "session_id"))?;
        let spans = if rows
            .iter()
            .any(|row| string(row, "parent_run_id").is_some())
        {
            rows.iter()
                .map(|row| TraceSpan {
                    id: string(row, "id").expect("validated run id").to_owned(),
                    parent_span_id: string(row, "parent_run_id").map(str::to_owned),
                    name: string(row, "name").map(str::to_owned),
                    started_at: string(row, "start_time").map(str::to_owned),
                })
                .collect()
        } else {
            Vec::new()
        };
        let run = Run {
            schema_version: SCHEMA_VERSION,
            id: format!("langsmith:{trace}"),
            source: project.map_or_else(|| "langsmith".to_owned(), |id| format!("langsmith:{id}")),
            organization_id,
            user_id,
            conversation_id: thread,
            calls,
            spans,
        };
        crate::validate(&run)?;
        runs.push(run);
    }
    if runs.is_empty() {
        return Err("the export contains no LLM or tool spans".into());
    }
    Ok(runs)
}

fn shared_field(
    rows: &[&Value],
    trace: &str,
    name: &str,
    read: impl for<'a> Fn(&'a Value) -> Option<&'a str>,
) -> Result<Option<String>, String> {
    let values: BTreeSet<_> = rows.iter().filter_map(|row| read(row)).collect();
    if values.len() > 1 {
        return Err(format!(
            "conflicting {name} identifiers in LangSmith trace {trace}"
        ));
    }
    Ok(values.first().map(|value| (*value).to_owned()))
}

fn new_call(row: &Value, trace: &str) -> Call {
    let id = string(row, "id").expect("validated run id");
    let metadata = &row["extra"]["metadata"];
    let mut call = empty_call(
        id,
        Some(trace.to_owned()),
        string(row, "start_time").map(str::to_owned),
    );
    call.turn_id = string(metadata, "turn_id").map(str::to_owned);
    call.model = string(metadata, "ls_model_name").map(str::to_owned);
    call
}

fn import_inference(row: &Value, call: &mut Call) -> Result<(), String> {
    let inputs = &row["inputs"];
    let outputs = &row["outputs"];
    let messages = message_list(&inputs["messages"], "inputs.messages", &call.id)?;
    for (index, message) in messages.iter().enumerate() {
        let field = message_field(messages.field, index, message);
        let data = message_data(message);
        let role = role(message, data);
        if let Some(content) = data.get("content").filter(|value| !value.is_null()) {
            let exact = format!("{field}.content");
            let items = content
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(content));
            for (part, item) in items.iter().enumerate() {
                let item_field = format!("{exact}[{part}]");
                let anchor = if content.is_array() {
                    &item_field
                } else {
                    &exact
                };
                let native_result = string(item, "type") == Some("tool_result");
                add_context(
                    call,
                    if role == Some("tool") || native_result {
                        "tool_result"
                    } else {
                        "message"
                    },
                    role,
                    item,
                    anchor,
                );
                if native_result {
                    if let Some(id) = string(item, "tool_use_id") {
                        call.tool_results.push(ToolResult {
                            tool_call_id: id.to_owned(),
                            outcome: item.get("content").cloned(),
                            evidence: evidence(call, anchor),
                        });
                    }
                }
            }
            if role == Some("tool") {
                if let Some(id) = string(data, "tool_call_id") {
                    call.tool_results.push(ToolResult {
                        tool_call_id: id.to_owned(),
                        outcome: Some(content.clone()),
                        evidence: evidence(call, &exact),
                    });
                }
            }
        }
    }
    if messages.is_empty() {
        import_prompt(inputs, call)?;
    }
    if let Some(tools) = inputs["tools"].as_array() {
        for (index, tool) in tools.iter().enumerate() {
            add_context(
                call,
                "tool_definition",
                None,
                tool,
                &format!("inputs.tools[{index}]"),
            );
        }
    }

    let messages = message_list(&outputs["messages"], "outputs.messages", &call.id)?;
    if !messages.is_empty() {
        for (index, message) in messages.iter().enumerate() {
            import_message_output(
                call,
                message_data(message),
                &message_field(messages.field, index, message),
            );
        }
    } else if let Some(generations) = outputs["generations"].as_array() {
        if generations.len() > 1 {
            return Err(format!(
                "batched outputs.generations in span {} need one prompt per exported run",
                call.id
            ));
        }
        if let Some(choices) = generations.first().and_then(Value::as_array) {
            for (index, choice) in choices.iter().enumerate() {
                let prefix = format!("outputs.generations[0][{index}]");
                if let Some(message) = choice.get("message").filter(|message| message.is_object()) {
                    import_message_output(
                        call,
                        message_data(message),
                        &format!("{prefix}.message{}", message_suffix(message)),
                    );
                } else if let Some(text) = choice.get("text").filter(|value| !value.is_null()) {
                    add_decision(
                        call,
                        "model_output",
                        "Model answer",
                        text.clone(),
                        &format!("{prefix}.text"),
                        None,
                    );
                }
            }
        }
    } else if let Some(choices) = outputs["choices"].as_array() {
        for (index, choice) in choices.iter().enumerate() {
            if let Some(message) = choice.get("message").filter(|message| message.is_object()) {
                import_message_output(call, message, &format!("outputs.choices[{index}].message"));
            } else if let Some(text) = choice.get("text").filter(|value| !value.is_null()) {
                add_decision(
                    call,
                    "model_output",
                    "Model answer",
                    text.clone(),
                    &format!("outputs.choices[{index}].text"),
                    None,
                );
            }
        }
    } else if let Some((field, value)) = ["output", "text"].into_iter().find_map(|field| {
        outputs
            .get(field)
            .filter(|value| !value.is_null())
            .map(|value| (field, value))
    }) {
        add_decision(
            call,
            "model_output",
            "Model answer",
            value.clone(),
            &format!("outputs.{field}"),
            None,
        );
    }
    Ok(())
}

fn import_prompt(inputs: &Value, call: &mut Call) -> Result<(), String> {
    if let Some(prompt) = inputs.get("prompt").filter(|value| !value.is_null()) {
        add_context(call, "prompt", None, prompt, "inputs.prompt");
    } else if let Some(prompts) = inputs["prompts"].as_array() {
        if prompts.len() > 1 {
            return Err(format!(
                "batched inputs.prompts in span {} need one prompt per exported run",
                call.id
            ));
        }
        if let Some(prompt) = prompts.first().filter(|value| !value.is_null()) {
            add_context(call, "prompt", None, prompt, "inputs.prompts[0]");
        }
    }
    Ok(())
}

fn import_message_output(call: &mut Call, data: &Value, field: &str) {
    if let Some(content) = data.get("content").filter(|value| {
        !value.is_null()
            && !value.as_str().is_some_and(|text| text.trim().is_empty())
            && !value.as_array().is_some_and(Vec::is_empty)
    }) {
        if let Some(parts) = content.as_array() {
            for (index, part) in parts.iter().enumerate() {
                if let Some(text) = part.get("text").filter(|text| !text.is_null()) {
                    add_decision(
                        call,
                        "model_output",
                        "Model answer",
                        text.clone(),
                        &format!("{field}.content[{index}].text"),
                        None,
                    );
                }
            }
        } else {
            add_decision(
                call,
                "model_output",
                "Model answer",
                content.clone(),
                &format!("{field}.content"),
                None,
            );
        }
    }
    let candidates = [
        (data["tool_calls"].as_array(), "tool_calls"),
        (
            data["additional_kwargs"]["tool_calls"].as_array(),
            "additional_kwargs.tool_calls",
        ),
    ];
    if let Some((Some(calls), path)) = candidates
        .into_iter()
        .find(|(calls, _)| calls.is_some_and(|calls| !calls.is_empty()))
    {
        for (index, tool) in calls.iter().enumerate() {
            let function = &tool["function"];
            let label = string(tool, "name")
                .or_else(|| string(function, "name"))
                .unwrap_or("Tool call");
            let arguments = tool
                .get("args")
                .or_else(|| function.get("arguments"))
                .cloned()
                .unwrap_or(Value::Null);
            add_decision(
                call,
                "tool_call",
                label,
                arguments,
                &format!("{field}.{path}[{index}]"),
                string(tool, "id"),
            );
        }
    } else if let Some(parts) = data["content"].as_array() {
        for (index, part) in parts.iter().enumerate() {
            if string(part, "type") == Some("tool_use") {
                add_decision(
                    call,
                    "tool_call",
                    string(part, "name").unwrap_or("Tool call"),
                    part["input"].clone(),
                    &format!("{field}.content[{index}]"),
                    string(part, "id"),
                );
            }
        }
    }
}

fn import_tool_span(row: &Value, call: &mut Call) {
    let label = string(row, "name").unwrap_or("Tool execution");
    if let Some(outputs) = row.get("outputs").filter(|value| !value.is_null()) {
        add_decision(
            call,
            "tool_execution",
            label,
            outputs.clone(),
            "outputs",
            None,
        );
        let id = string(&row["extra"]["metadata"], "tool_call_id")
            .or_else(|| string(message_data(&outputs["output"]), "tool_call_id"));
        if let Some(id) = id {
            call.tool_results.push(ToolResult {
                tool_call_id: id.to_owned(),
                outcome: Some(outputs.clone()),
                evidence: evidence(call, "outputs"),
            });
        }
    }
}

struct Messages<'a> {
    values: &'a [Value],
    field: &'static str,
}

impl<'a> std::ops::Deref for Messages<'a> {
    type Target = [Value];

    fn deref(&self) -> &Self::Target {
        self.values
    }
}

fn message_list<'a>(
    value: &'a Value,
    field: &'static str,
    call_id: &str,
) -> Result<Messages<'a>, String> {
    let Some(array) = value.as_array() else {
        return Ok(Messages { values: &[], field });
    };
    if let Some(first) = array.first().and_then(Value::as_array) {
        if array.len() > 1 {
            return Err(format!(
                "batched {field} in span {call_id} needs one prompt per exported run"
            ));
        }
        let field = if field == "inputs.messages" {
            "inputs.messages[0]"
        } else {
            "outputs.messages[0]"
        };
        return Ok(Messages {
            values: first,
            field,
        });
    }
    Ok(Messages {
        values: array,
        field,
    })
}

fn message_field(field: &str, index: usize, message: &Value) -> String {
    format!("{field}[{index}]{}", message_suffix(message))
}

fn message_suffix(message: &Value) -> &'static str {
    if message["kwargs"].is_object() {
        ".kwargs"
    } else if message["data"].is_object() {
        ".data"
    } else {
        ""
    }
}

fn message_data(message: &Value) -> &Value {
    match message_suffix(message) {
        ".kwargs" => &message["kwargs"],
        ".data" => &message["data"],
        _ => message,
    }
}

fn role<'a>(message: &'a Value, data: &'a Value) -> Option<&'a str> {
    let name = string(data, "role")
        .or_else(|| string(message, "type").filter(|role| *role != "constructor"))
        .or_else(|| {
            message["id"]
                .as_array()
                .and_then(|id| id.last())
                .and_then(Value::as_str)
        });
    match name {
        Some("human" | "HumanMessage" | "HumanMessageChunk") => Some("user"),
        Some("ai" | "AIMessage" | "AIMessageChunk") => Some("assistant"),
        Some("system" | "SystemMessage" | "SystemMessageChunk") => Some("system"),
        Some("tool" | "ToolMessage" | "ToolMessageChunk") => Some("tool"),
        other => other,
    }
}

fn add_context(call: &mut Call, kind: &str, role: Option<&str>, value: &Value, field: &str) {
    call.context.push(ContextItem {
        id: field.to_owned(),
        kind: kind.to_owned(),
        role: role.map(str::to_owned),
        text: Some(
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| crate::canonical_json(value)),
        ),
        content_reference: None,
        document_identity: None,
        request_fragment: None,
        source_id: None,
        source_version: None,
        authority: None,
        availability: Availability::Included,
        evidence: evidence(call, field),
        rule: None,
    });
}

fn add_decision(
    call: &mut Call,
    kind: &str,
    label: &str,
    value: Value,
    field: &str,
    tool_call_id: Option<&str>,
) {
    call.decisions.push(Decision {
        id: field.to_owned(),
        kind: kind.to_owned(),
        label: label.to_owned(),
        outcome: value,
        evidence: evidence(call, field),
        tool_call_id: tool_call_id.map(str::to_owned),
        applies_to: None,
    });
}

fn evidence(call: &Call, field: &str) -> Evidence {
    Evidence {
        span_id: call.span_id.clone(),
        field: field.to_owned(),
        message_index: None,
        part_index: None,
    }
}

fn string<'a>(object: &'a Value, field: &str) -> Option<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}
