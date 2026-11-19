//! Paginate activation evidence for an agent within the selected trace scope.

use super::{group, Group, Occurrence};
use crate::{
    activation::Report,
    conflict_query::{timestamp, Scope, TimeWindow},
    turn_evidence, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    block: usize,
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    source_id: Option<String>,
    checkpoint: Option<String>,
    after: Option<String>,
    before: Option<String>,
    group: Option<String>,
    #[serde(default)]
    include_excerpts: bool,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Thread {
    trace_source: String,
    organization_id: Option<String>,
    user_id: Option<String>,
    conversation_id: String,
}

impl Thread {
    pub(crate) fn includes(&self, run: &Run) -> bool {
        run.source == self.trace_source
            && run.organization_id == self.organization_id
            && run.user_id == self.user_id
            && run.conversation_id.as_deref() == Some(self.conversation_id.as_str())
    }
}

pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    reports: &[Report],
    allow_excerpts: bool,
) -> Result<Value, String> {
    if request.version != 1 {
        return Err("Activation effect queries require version 1.".into());
    }
    if !(1..=50).contains(&request.limit) {
        return Err("Choose a page size from 1 to 50.".into());
    }
    if request.include_excerpts && !allow_excerpts {
        return Err("This MCP process has not enabled excerpts.".into());
    }
    let window = TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?;
    let scoped: Vec<_> = runs
        .into_iter()
        .filter(|run| {
            request.scope.includes(run)
                && request
                    .thread
                    .as_ref()
                    .is_none_or(|thread| thread.includes(run))
        })
        .collect();
    let mut groups: Vec<_> = group(&scoped, reports)
        .into_iter()
        .filter(|entry| {
            request.block < entry.comparison.layer_count
                && request
                    .source_id
                    .as_deref()
                    .is_none_or(|id| entry.comparison.source_id == Some(id))
                && request
                    .checkpoint
                    .as_deref()
                    .is_none_or(|label| entry.comparison.checkpoint.label == label)
        })
        .collect();
    if request.after.is_some() || request.before.is_some() {
        groups.retain_mut(|group| {
            group.occurrences.retain(|item| {
                window.includes(item.started_at.and_then(|value| timestamp(value).ok()))
            });
            !group.occurrences.is_empty()
        });
    }

    if let Some(key) = &request.group {
        let mut entry = None;
        for candidate in &groups {
            if group_key(candidate)? == *key {
                entry = Some(candidate);
                break;
            }
        }
        let entry = entry.ok_or("The activation group is unavailable for this scope and block.")?;
        let items = &entry.occurrences;
        let rows: Vec<_> = items
            .iter()
            .skip(request.offset)
            .take(request.limit)
            .map(|item| evidence(item, request.block))
            .collect();
        return Ok(json!({
            "kind": "activation_evidence",
            "block": request.block,
            "captured_excerpts_enabled": allow_excerpts,
            "group": summarize(entry, key, request.block, request.include_excerpts),
            "total_occurrences": items.len(),
            "occurrences": rows,
            "next_offset": next_offset(request, rows.len(), items.len()),
        }));
    }
    let rows: Vec<_> = groups
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|entry| {
            Ok(summarize(
                entry,
                &group_key(entry)?,
                request.block,
                request.include_excerpts,
            ))
        })
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "kind": "activation_groups",
        "block": request.block,
        "captured_excerpts_enabled": allow_excerpts,
        "total_groups": groups.len(),
        "groups": rows,
        "next_offset": next_offset(request, rows.len(), groups.len()),
    }))
}

fn next_offset(request: &Request, page_count: usize, total: usize) -> Option<usize> {
    let next = request.offset.saturating_add(page_count);
    (next < total).then_some(next)
}

fn group_key(group: &Group<'_>) -> Result<String, String> {
    let bytes = serde_json::to_vec(&group.comparison).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn summarize(group: &Group<'_>, key: &str, block: usize, include_excerpts: bool) -> Value {
    let comparison = &group.comparison;
    let mut decisions = BTreeMap::new();
    let mut conversations = BTreeSet::new();
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for item in &group.occurrences {
        if let Some(conversation) = item.conversation_id {
            conversations.insert((
                item.trace_source,
                item.organization_id,
                item.user_id,
                conversation,
            ));
        }
        let patched = item.patched_margins[block];
        let change = patched - item.baseline_margin;
        minimum = minimum.min(change);
        maximum = maximum.max(change);
        let entry = decisions.entry(item.location).or_insert((true, true));
        entry.0 &= change < 0.0;
        entry.1 &= item.baseline_margin > 0.0 && patched < 0.0;
    }
    let mut result = json!({
        "key": key, "model_id": comparison.model_id, "model_revision": comparison.model_revision,
        "runtime": comparison.runtime, "layer_count": comparison.layer_count,
        "decision_kind": comparison.decision_kind, "decision_label": comparison.decision_label,
        "source_id": comparison.source_id, "source_version": comparison.source_version,
        "checkpoint": comparison.checkpoint.label, "output_token": comparison.checkpoint.output_token,
        "decision_count": decisions.len(), "measurement_count": group.occurrences.len(),
        "conversation_count": conversations.len(),
        "lower_score_count": decisions.values().filter(|(lower, _)| *lower).count(),
        "flipped_count": decisions.values().filter(|(_, flipped)| *flipped).count(),
        "change_log_odds": { "minimum": minimum, "maximum": maximum },
    });
    if include_excerpts {
        result["contrasts"] = json!({
            "before": turn_evidence::excerpt(comparison.before),
            "after": turn_evidence::excerpt(comparison.after),
            "alternatives": comparison.checkpoint.alternatives.each_ref().map(|text| turn_evidence::excerpt(text)),
        });
    }
    result
}

fn evidence(item: &Occurrence<'_>, block: usize) -> Value {
    let location = item.location;
    let path = crate::activation::decision_path(location);
    let context = utf8_percent_encode(item.context_id, NON_ALPHANUMERIC);
    let checkpoint = item.checkpoint_index;
    let patched = item.patched_margins[block];
    json!({
        "location": location, "context_id": item.context_id, "report_id": item.report_id,
        "checkpoint_index": checkpoint, "trace_source": item.trace_source,
        "organization_id": item.organization_id, "user_id": item.user_id,
        "conversation_id": item.conversation_id, "turn_id": item.turn_id, "started_at": item.started_at,
        "original_log_odds": item.baseline_margin, "patched_log_odds": patched,
        "change_log_odds": patched - item.baseline_margin,
        "inspector_path": crate::activation::activation_path(location, item.report_id, "checkpoint", checkpoint, block),
        "source_inspector_path": format!("{path}&context={context}"),
    })
}
