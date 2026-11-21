use a2context::{Call, Run, ToolResult};
use serde::Serialize;
use std::collections::BTreeMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Serialize)]
pub(super) struct InputResult<'a> {
    context: &'a str,
    invocation: &'a str,
    execution: Option<&'a str>,
    tool: Option<&'a str>,
    exact_payload_match: Option<bool>,
}

pub(super) fn received_by<'a>(run: &'a Run, request: &'a Call) -> Vec<InputResult<'a>> {
    let started = timestamp(request.started_at.as_deref());
    let mut executions: BTreeMap<&str, Option<(&Call, &ToolResult, &str)>> = BTreeMap::new();
    for call in run.calls.iter().filter(|call| call.id != request.id) {
        for result in &call.tool_results {
            let mut decisions = call.decisions.iter().filter(|decision| {
                decision.kind == "tool_execution"
                    && decision.evidence.span_id == result.evidence.span_id
            });
            if let Some(decision) = decisions.next().filter(|_| decisions.next().is_none()) {
                executions
                    .entry(&result.tool_call_id)
                    .and_modify(|previous| *previous = None)
                    .or_insert(Some((call, result, &decision.label)));
            }
        }
    }

    request
        .tool_results
        .iter()
        .filter_map(|result| {
            let context = request
                .context
                .iter()
                .find(|item| item.evidence == result.evidence)?;
            let execution = executions
                .get(result.tool_call_id.as_str())
                .copied()
                .flatten()
                .filter(|(call, _, _)| {
                    timestamp(call.ended_at.as_deref())
                        .zip(started)
                        .is_some_and(|(end, start)| end <= start)
                });
            Some(InputResult {
                context: &context.id,
                invocation: &result.tool_call_id,
                execution: execution.map(|(call, _, _)| call.id.as_str()),
                tool: execution.map(|(_, _, tool)| tool),
                exact_payload_match: execution.and_then(|(_, source, _)| {
                    let returned = source.outcome.as_ref().filter(|value| !value.is_null())?;
                    let received = result.outcome.as_ref().filter(|value| !value.is_null())?;
                    Some(returned == received)
                }),
            })
        })
        .collect()
}

fn timestamp(value: Option<&str>) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value?, &Rfc3339).ok()
}
