use super::{
    collect_result_values, display_value, exact_token, identifier, result_body, result_invocation,
    visit_values, Call, ContextItem, ContextLink, Decision, LinkKind, ResultValue, ToolOrigin,
};
use crate::Availability;
use serde_json::Value;
use std::collections::BTreeMap;

struct CapturedValues<'a> {
    item: &'a ContextItem,
    invocation_id: Option<&'a str>,
    values: Vec<ResultValue>,
}

pub(super) fn link_outcomes<'a>(
    run_id: &'a str,
    call: &'a Call,
    earlier_tools: &BTreeMap<&'a str, Option<ToolOrigin<'a>>>,
    links: &mut Vec<ContextLink<'a>>,
) {
    let mut decisions = call
        .decisions
        .iter()
        .filter(|decision| {
            matches!(
                decision.kind.as_str(),
                "model_output" | "decision_model" | "guardrail"
            ) && !decision.outcome.is_null()
        })
        .peekable();
    if decisions.peek().is_none() {
        return;
    }
    let sources: Vec<_> = call
        .context
        .iter()
        .filter_map(|item| captured_values(call, item))
        .collect();
    if sources.is_empty() {
        return;
    }
    for decision in decisions {
        find_outcome_matches(decision, &sources, |source, value, kind, outcome_path| {
            links.push(ContextLink {
                run_id,
                call_id: &call.id,
                decision_id: &decision.id,
                context_id: &source.item.id,
                kind,
                argument_path: None,
                outcome_path,
                context_path: Some(value.path.clone()),
                excerpt: display_value(&value.value),
                source_tool_call_id: source.invocation_id,
                source_tool: source
                    .invocation_id
                    .and_then(|id| earlier_tools.get(id).copied().flatten()),
            });
        });
    }
}

fn captured_values<'a>(call: &'a Call, item: &'a ContextItem) -> Option<CapturedValues<'a>> {
    if item.availability != Availability::Included {
        return None;
    }
    let text = item.text.as_deref()?;
    let (body, invocation_id) = match item.kind.as_str() {
        "tool_call_response" | "tool_result" => {
            (result_body(&item.kind, text), result_invocation(call, item))
        }
        "input_document" | "request" | "request_field" => (serde_json::from_str(text).ok()?, None),
        _ => return None,
    };
    if !body.is_object() && !body.is_array() {
        return None;
    }
    let mut values = Vec::new();
    collect_result_values(&body, "", None, &mut values);
    (!values.is_empty()).then_some(CapturedValues {
        item,
        invocation_id,
        values,
    })
}

fn find_outcome_matches<'a>(
    decision: &Decision,
    sources: &[CapturedValues<'a>],
    mut record: impl FnMut(&CapturedValues<'a>, &ResultValue, LinkKind, Option<String>),
) {
    let decoded = decision
        .outcome
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let value = decoded.as_ref().unwrap_or(&decision.outcome);
    let fields = sources
        .iter()
        .flat_map(|source| source.values.iter().map(move |value| (source, value)));
    if value.is_object() || value.is_array() {
        visit_values(value, "", None, &mut |outcome, path, key| {
            let Some(key) = key else {
                return;
            };
            for (source, field) in fields.clone() {
                if field.key.as_deref() == Some(key) && field.value == *outcome {
                    record(source, field, LinkKind::OutcomeMatch, Some(path.to_owned()));
                }
            }
        });
    } else if value.is_number() {
        let mut matches = fields.filter(|(_, field)| field.key.is_some() && field.value == *value);
        if let (Some((source, field)), None) = (matches.next(), matches.next()) {
            record(source, field, LinkKind::OutcomeMatch, Some(String::new()));
        }
    } else if let Some(text) = decision.outcome.as_str() {
        for (source, field) in fields {
            if let Some(candidate) = field
                .value
                .as_str()
                .filter(|candidate| identifier(candidate))
            {
                if exact_token(text, candidate) {
                    record(source, field, LinkKind::OutcomeIdentifier, None);
                }
            }
        }
    }
}
