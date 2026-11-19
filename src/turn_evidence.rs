//! Paginated evidence for a recorded decision and its turn.

use crate::{
    activation, context_screening, inspect, preflight::Verification, replay, Call, ContextItem, Run,
};
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
    let preflight = inspection
        .preflight_checks
        .iter()
        .find(|check| check.call_id == call.id);
    let preflight_report = preflight.and_then(|check| match &check.verification {
        Verification::Matched { report } => Some(report),
        Verification::Unverified { .. } => None,
    });
    let preflight_summary = preflight.map(|check| match &check.verification {
        Verification::Matched { report } => json!({
            "status": "matched", "anchor": check.evidence, "scope": check.scope,
            "finding_count": report.findings.len(), "scoped_rule_items": report.scoped_rule_items,
        }),
        Verification::Unverified { issue } => json!({
            "status": "unverified", "anchor": check.evidence, "issue": issue,
        }),
    });
    let candidate_probes = inspection
        .candidate_probes
        .iter()
        .filter(|probe| probe.call_id == call.id);
    let candidate_summary: Vec<_> = candidate_probes
        .clone()
        .map(|probe| candidate_summary(probe, include_excerpts))
        .collect();
    let shadow_probes = inspection
        .shadow_probes
        .iter()
        .filter(|probe| probe.call_id == call.id);
    let shadow_summary: Vec<_> = shadow_probes
        .clone()
        .map(|probe| shadow_summary(probe, include_excerpts))
        .collect();
    let screen = inspection
        .context_screens
        .iter()
        .find(|screen| screen.call_id == call.id);
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
    let mut rows = preflight_report
        .into_iter()
        .flat_map(|report| preflight_rows(report, include_excerpts))
        .chain(candidate_probes.flat_map(|probe| candidate_rows(probe, include_excerpts)))
        .chain(shadow_probes.flat_map(|probe| shadow_rows(probe, include_excerpts)))
        .chain(finding_rows)
        .chain(screen_rows(screen, call, include_excerpts))
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

    let mut result = json!({
        "version": 1, "kind": "turn_evidence", "run_id": run.id, "call_id": call.id, "turn_id": call.turn_id, "started_at": call.started_at,
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
        "counts": counts, "recorded_tool_result": recorded_tool_result, "preflight": preflight_summary,
        "captured_excerpts_enabled": allow_excerpts,
        "retrieval_label": retrieval.map(|retrieval| &retrieval.label),
        "original_request_captured": call.recorded_request.is_some(),
        "inspector_path": inspector_path(&run.id, &call.id, decision_id), "rows": rows, "next_offset": next,
    });
    if !candidate_summary.is_empty() {
        result["candidate_probes"] = json!(candidate_summary);
    }
    if !shadow_summary.is_empty() {
        result["shadow_probes"] = json!(shadow_summary);
    }
    if let Some(screen) = screen {
        result["context_screening"] = screen_summary(screen);
    }
    Ok(result)
}

pub(crate) fn preflight_rows(
    report: &crate::preflight::Report,
    include_excerpts: bool,
) -> impl Iterator<Item = Value> + '_ {
    report.findings.iter().map(move |finding| json!({
        "kind": "preflight_finding", "finding": finding.kind, "rule": finding.key, "scope": finding.scope,
        "inputs": finding.sources.iter().map(|source| json!({
            "context_id": source.context_id, "source_id": source.source_id,
            "source_version": source.source_version, "anchor": source.evidence,
            "request_fragment": source.request_fragment,
            "rule_value": source.value.as_ref().filter(|_| include_excerpts),
        })).collect::<Vec<_>>(),
    }))
}

pub(crate) fn screen_summary(screen: &context_screening::Recorded<'_>) -> Value {
    match &screen.verification {
        context_screening::Verification::Matched { report } => json!({
            "status": "matched", "anchor": screen.evidence, "scorer": report.scorer,
            "compared_pairs": report.pairs.len(), "total_pairs": report.total_pairs,
            "flagged_pairs": report.pairs.iter().filter(|pair| pair.suggestion.is_some()).count(),
            "unscored_pairs": report.pairs.iter().filter(|pair| matches!(pair.assessment, context_screening::Assessment::Unscored { .. })).count(),
        }),
        context_screening::Verification::Unverified { issue } => json!({
            "status": "unverified", "anchor": screen.evidence, "issue": issue,
        }),
    }
}

pub(crate) fn screen_rows<'a>(
    screen: Option<&context_screening::Recorded<'a>>,
    call: &'a Call,
    include_excerpts: bool,
) -> impl Iterator<Item = Value> + 'a {
    let screen_report = screen.and_then(|screen| match &screen.verification {
        context_screening::Verification::Matched { report } => Some(*report),
        context_screening::Verification::Unverified { .. } => None,
    });
    screen_report.into_iter().flat_map(move |report| {
        let flagged = report.pairs.iter().filter(|pair| pair.suggestion.is_some());
        let remaining = report.pairs.iter().filter(|pair| pair.suggestion.is_none());
        flagged.chain(remaining).map(move |pair| json!({
            "kind": "context_screen_pair", "suggestion": pair.suggestion, "assessment": pair.assessment,
            "inputs": pair.sources.iter().filter_map(|&index| {
                let id = &report.sources[index].context_id;
                call.context.iter().find(|item| &item.id == id).map(|item| context_ref(item, include_excerpts))
            }).collect::<Vec<_>>(),
        }))
    })
}

pub(crate) fn candidate_summary(
    probe: &replay::candidates::RecordedProbe<'_>,
    include_excerpts: bool,
) -> Value {
    use replay::candidates::Verification;
    match &probe.verification {
        Verification::Matched { measurement } => json!({
            "report_id": probe.report_id, "status": "matched", "anchor": probe.evidence,
            "candidate_ids": measurement.candidates.each_ref().map(|candidate| &candidate.id),
            "model_id": measurement.model_id, "model_revision": measurement.model_revision,
            "runtime": measurement.runtime, "scorer_id": measurement.scorer_id,
            "baseline_log_odds": measurement.baseline_log_odds, "effect_count": measurement.effects.len(),
            "prefix_excerpt": include_excerpts.then(|| excerpt(measurement.assistant_prefix)),
            "candidate_excerpts": include_excerpts.then(|| measurement.candidates.each_ref().map(|candidate| excerpt(&candidate.continuation))),
        }),
        Verification::Unverified { issue } => json!({
            "report_id": probe.report_id, "status": "unverified", "anchor": probe.evidence, "issue": issue,
        }),
    }
}

pub(crate) fn candidate_rows<'a>(
    probe: &'a replay::candidates::RecordedProbe<'_>,
    include_excerpts: bool,
) -> impl Iterator<Item = Value> + 'a {
    use replay::candidates::Verification;
    let report = match &probe.verification {
        Verification::Matched { measurement } => Some(measurement),
        Verification::Unverified { .. } => None,
    };
    report.into_iter().flat_map(move |measurement| measurement.effects.iter().map(move |effect| json!({
        "kind": "candidate_probe_effect", "report_id": probe.report_id,
        "context_id": effect.context_id, "source_id": effect.source_id, "source_version": effect.source_version,
        "anchor": effect.evidence, "pointer": effect.pointer, "text_range": effect.text_range,
        "operation": effect.operation, "replacement_excerpt": include_excerpts.then(|| excerpt(effect.replacement)),
        "log_odds": effect.log_odds, "change_log_odds": effect.change_log_odds,
        "candidate_ids": measurement.candidates.each_ref().map(|candidate| &candidate.id),
    })))
}

pub(crate) fn shadow_summary(
    probe: &replay::shadow::RecordedProbe<'_>,
    include_excerpts: bool,
) -> Value {
    use replay::shadow::Verification;
    match &probe.verification {
        Verification::Matched { measurement } => json!({
            "report_id": probe.report_id, "status": "matched", "anchor": probe.evidence,
            "runner_id": measurement.runner_id, "sampling_design": measurement.sampling_design,
            "baseline": measurement.baseline, "effect_count": measurement.effects.len(),
            "pair": measurement.pair, "control_comparison": measurement.control_comparison,
            "expected_kind": measurement.expected.kind,
            "match_rule": include_excerpts.then_some(measurement.match_rule),
            "expected_label": include_excerpts.then(|| excerpt(&measurement.expected.label)),
            "expected_outcome": include_excerpts.then(|| match &measurement.expected.outcome {
                Value::String(text) => excerpt(text),
                value => excerpt(&value.to_string()),
            }),
        }),
        Verification::Unverified { issue } => json!({
            "report_id": probe.report_id, "status": "unverified", "anchor": probe.evidence, "issue": issue,
        }),
    }
}

pub(crate) fn shadow_rows<'a>(
    probe: &'a replay::shadow::RecordedProbe<'_>,
    include_excerpts: bool,
) -> impl Iterator<Item = Value> + 'a {
    use replay::shadow::Verification;
    let effects = match &probe.verification {
        Verification::Matched { measurement } => measurement.effects.as_slice(),
        Verification::Unverified { .. } => &[],
    };
    effects.iter().map(move |effect| {
        let edit = effect.intervention;
        json!({
            "kind": "shadow_probe_effect", "report_id": probe.report_id,
            "context_id": effect.context_id, "source_id": effect.source_id, "source_version": effect.source_version,
            "anchor": effect.evidence, "pointer": edit.pointer, "text_range": edit.text_range,
            "operation": edit.operation, "replacement_excerpt": include_excerpts.then(|| excerpt(&edit.replacement)),
            "summary": effect.summary, "difference_pp": effect.difference_pp,
            "difference_95_interval_pp": effect.difference_95_interval_pp,
        })
    })
}

pub(crate) fn inspector_path(run: &str, call: &str, decision: Option<&str>) -> String {
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

pub(crate) fn context_ref(item: &ContextItem, include_excerpts: bool) -> Value {
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
        "context_ids": report.variants.iter().map(|variant| &variant.intervention.context_id).collect::<Vec<_>>(),
        "summary": pair.summary, "difference_pp": pair.difference_pp,
        "difference_95_interval_pp": pair.difference_95_interval_pp, "interaction_pp": pair.interaction_pp,
    })))).chain(report.control_comparison.as_ref().map(move |control| row("negative_control", json!(control))))
}
