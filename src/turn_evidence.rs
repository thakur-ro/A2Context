//! Paginated evidence for a recorded decision and its turn.

use crate::{activation, inspect, replay, Call, ContextItem, Run};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub run_id: String,
    pub call_id: String,
    pub decision_id: Option<String>,
    #[serde(default)]
    pub include_excerpts: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

/// Query only the runs and replay reports the caller is authorized to read.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    replays: &[replay::Replay],
    allow_excerpts: bool,
) -> Result<Value, String> {
    if request.include_excerpts && !allow_excerpts {
        return Err("Captured excerpts are disabled for this MCP server.".into());
    }
    if !(1..=50).contains(&request.limit) {
        return Err("Choose a page size from 1 to 50.".into());
    }
    let unavailable = "The recorded run, call, or decision is unavailable.";
    let runs: Vec<_> = runs.into_iter().collect();
    let run = runs
        .iter()
        .copied()
        .find(|run| run.id == request.run_id)
        .ok_or(unavailable)?;
    let call = run
        .calls
        .iter()
        .find(|call| call.id == request.call_id)
        .ok_or(unavailable)?;
    let decision = match request.decision_id.as_deref() {
        Some(id) => Some(
            call.decisions
                .iter()
                .find(|decision| decision.id == id)
                .ok_or(unavailable)?,
        ),
        None => (call.decisions.len() == 1).then(|| &call.decisions[0]),
    };
    let decision_id = decision.map(|decision| decision.id.as_str());
    let include_excerpts = request.include_excerpts;
    let inspection = inspect(std::slice::from_ref(run))?;
    let at_decision = |call_id: &str, id: &str| call_id == call.id && decision_id == Some(id);
    let findings = inspection
        .findings
        .iter()
        .filter(|item| at_decision(&item.call_id, &item.decision_id));
    let links = inspection
        .context_links
        .iter()
        .filter(|item| at_decision(item.call_id, item.decision_id));
    let annotations = call
        .relationships
        .iter()
        .filter(|item| decision_id == Some(item.decision_id.as_str()));
    let reports = replays.iter().filter(|item| {
        item.location.run_id == run.id
            && at_decision(&item.location.call_id, &item.location.decision_id)
    });
    let recorded_tool_result = inspection.tool_links.iter()
        .find(|item| at_decision(item.call_id, item.decision_id))
        .map(|item| json!({
            "call_id": item.result_call_id, "tool_call_id": item.result.tool_call_id,
            "anchor": item.result.evidence, "outcome_captured": item.result.outcome.as_ref().is_some_and(|result| !result.is_null()),
        }));
    let steps = turn_calls(&runs, run, call);
    let turn_order = if steps.iter().all(|(_, _, time)| time.is_some()) {
        "captured_time"
    } else {
        "recorded_order"
    };
    let retrieval = call.retrieval.as_ref();
    let counts = json!({
        "rule_findings": findings.clone().count(),
        "context_links": links.clone().count() + annotations.clone().count(),
        "replay_tests": reports.clone().map(|report| report.variants.len() + usize::from(report.pair.is_some())
            + usize::from(report.control_comparison.is_some())).sum::<usize>(),
        "recorded_calls": steps.len(),
        "captured_inputs": call.context.len(),
        "retrieved_documents": retrieval.map_or(0, |retrieval| retrieval.documents.len()),
    });

    let finding_rows = findings.map(|finding| json!({
        "kind": "rule_finding", "finding": finding.kind, "rule": finding.key, "scope": finding.scope,
        "inputs": finding.context_ids.iter().filter_map(|id| call.context.iter().find(|item| &item.id == id))
            .map(|item| context_ref(item, include_excerpts)).collect::<Vec<_>>(),
    }));
    let annotation_rows = annotations.map(|item| {
        json!({
            "kind": "context_link", "relationship": "annotation", "context_id": item.context_id,
            "label": item.relation, "basis": item.basis, "anchor": item.evidence,
            "excerpt": item.excerpt.as_deref().filter(|_| include_excerpts).map(excerpt),
        })
    });
    let link_rows = links.map(|link| {
        json!({
            "kind": "context_link", "relationship": link.kind, "context_id": link.context_id,
            "context_path": link.context_path, "argument_path": link.argument_path,
            "outcome_path": link.outcome_path, "source_tool": link.source_tool,
            "excerpt": include_excerpts.then(|| excerpt(&link.excerpt)),
        })
    });
    let replay_rows = reports.flat_map(replay_tests);
    let decision_rows = call.decisions.iter().filter(|_| decision.is_none()).map(|item| {
        json!({
            "kind": "recorded_decision", "decision_id": item.id, "decision_kind": item.kind,
            "label": item.label, "anchor": item.evidence, "outcome_captured": !item.outcome.is_null(),
            "inspector_path": inspector_path(&run.id, &call.id, Some(&item.id)),
        })
    });
    let step_rows = steps.into_iter().map(|(run, step, _)| {
        json!({
            "kind": "recorded_call", "run_id": run.id, "call_id": step.id,
            "started_at": step.started_at, "decision_count": step.decisions.len(),
            "inspector_path": inspector_path(&run.id, &step.id, None),
        })
    });
    let context_rows = call.context.iter().map(|item| {
        json!({
            "kind": "captured_input", "input": context_ref(item, include_excerpts),
        })
    });
    let document_rows = retrieval
        .into_iter()
        .flat_map(|retrieval| &retrieval.documents)
        .map(|item| {
            json!({
                "kind": "retrieved_document", "input": context_ref(item, include_excerpts),
                "document_identity": item.document_identity,
            })
        });
    let mut rows = finding_rows
        .chain(annotation_rows)
        .chain(link_rows)
        .chain(replay_rows)
        .chain(decision_rows)
        .chain(step_rows)
        .chain(context_rows)
        .chain(document_rows)
        .skip(request.offset)
        .take(request.limit + 1)
        .collect::<Vec<_>>();
    let next = (rows.len() > request.limit).then(|| request.offset.saturating_add(request.limit));
    rows.truncate(request.limit);

    Ok(json!({
        "version": 1, "run_id": run.id, "call_id": call.id, "turn_id": call.turn_id, "started_at": call.started_at,
        "turn_order": turn_order, "trace_source": run.source,
        "organization_id": run.organization_id, "user_id": run.user_id, "conversation_id": run.conversation_id,
        "decision": decision.map(|item| json!({
            "id": item.id, "kind": item.kind, "label": item.label, "anchor": item.evidence,
            "outcome_captured": !item.outcome.is_null(), "declared_scope": item.applies_to,
            "outcome_excerpt": (include_excerpts && !item.outcome.is_null()).then(|| {
                match item.outcome.as_str() {
                    Some(text) => excerpt(text),
                    None => excerpt(&item.outcome.to_string()),
                }
            }),
        })),
        "counts": counts, "recorded_tool_result": recorded_tool_result,
        "captured_excerpts_enabled": allow_excerpts,
        "retrieval_label": retrieval.map(|retrieval| &retrieval.label),
        "original_request_captured": call.recorded_request.is_some(),
        "inspector_path": inspector_path(&run.id, &call.id, decision_id), "rows": rows, "next_offset": next,
    }))
}

fn inspector_path(run: &str, call: &str, decision: Option<&str>) -> String {
    let run = utf8_percent_encode(run, NON_ALPHANUMERIC);
    let call = utf8_percent_encode(call, NON_ALPHANUMERIC);
    let mut path = format!("/#v=1&view=conversation&run={run}&call={call}");
    if let Some(decision) = decision {
        path.push_str(&format!(
            "&decision={}",
            utf8_percent_encode(decision, NON_ALPHANUMERIC)
        ));
    }
    path
}

pub fn query_with_activations<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    replays: &[replay::Replay],
    activations: &[activation::Report],
    allow_excerpts: bool,
) -> Result<Value, String> {
    let mut result = query(request, runs, replays, allow_excerpts)?;
    if let Some(id) = result["decision"]["id"].as_str() {
        let scores = activation::output_scores::for_decision(
            activations,
            &request.run_id,
            &request.call_id,
            id,
        );
        let branches = activation::generation_branches::for_decision(
            activations,
            &request.run_id,
            &request.call_id,
            id,
        );
        let visibility = activation::visibility::query::for_decision(
            activations,
            &request.run_id,
            &request.call_id,
            id,
        );
        if !scores.is_empty() {
            result["output_score_reports"] = json!(scores);
        }
        if !branches.is_empty() {
            result["generation_branch_reports"] = json!(branches);
        }
        if !visibility.is_empty() {
            result["source_visibility_reports"] = json!(visibility);
        }
    }
    Ok(result)
}

fn turn_calls<'a>(
    runs: &[&'a Run],
    selected_run: &'a Run,
    selected_call: &'a Call,
) -> Vec<(&'a Run, &'a Call, Option<OffsetDateTime>)> {
    let Some(turn) = selected_call
        .turn_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    else {
        return vec![(selected_run, selected_call, None)];
    };
    let same_thread = |run: &Run| {
        run.id == selected_run.id
            || (selected_run.conversation_id.is_some()
                && run.conversation_id == selected_run.conversation_id
                && run.source == selected_run.source
                && run.organization_id == selected_run.organization_id
                && run.user_id == selected_run.user_id)
    };
    let mut steps: Vec<_> = runs
        .iter()
        .copied()
        .filter(|run| same_thread(run))
        .flat_map(|run| {
            run.calls
                .iter()
                .filter(move |call| call.turn_id.as_deref() == Some(turn))
                .map(move |call| {
                    let time = call
                        .started_at
                        .as_deref()
                        .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok());
                    (run, call, time)
                })
        })
        .collect();
    if steps.iter().all(|(_, _, time)| time.is_some()) {
        steps.sort_by_key(|(_, _, time)| *time);
    }
    steps
}

fn context_ref(item: &ContextItem, include_excerpts: bool) -> Value {
    json!({
        "context_id": item.id, "kind": item.kind, "role": item.role, "availability": item.availability,
        "source_id": item.source_id, "source_version": item.source_version, "anchor": item.evidence,
        "excerpt": item.text.as_deref().filter(|_| include_excerpts).map(excerpt),
    })
}

pub(crate) fn excerpt(text: &str) -> Value {
    let end = text.char_indices().nth(600).map(|(index, _)| index);
    json!({"text": &text[..end.unwrap_or(text.len())], "truncated": end.is_some()})
}

fn replay_tests(report: &replay::Replay) -> impl Iterator<Item = Value> + '_ {
    let row = move |test: &str, evidence: Value| {
        json!({
            "kind": "replay_test", "test": test, "report_id": report.id, "runner_id": report.runner_id,
            "baseline": report.baseline_summary, "match_rule": report.match_rule,
            "sampling_design": report.sampling_design, "result": evidence,
        })
    };
    report.variants.iter().map(move |variant| row("individual", json!({
        "context_id": variant.intervention.context_id, "operation": variant.intervention.operation,
        "summary": variant.summary, "difference_pp": variant.difference_pp,
        "difference_95_interval_pp": variant.difference_95_interval_pp,
    }))).chain(report.pair.as_ref().map(move |pair| row("pair", json!({
        "summary": pair.summary, "difference_pp": pair.difference_pp,
        "difference_95_interval_pp": pair.difference_95_interval_pp, "interaction_pp": pair.interaction_pp,
    })))).chain(report.control_comparison.as_ref().map(move |control| row("negative_control", json!(control))))
}
