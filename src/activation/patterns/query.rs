//! Paginate activation evidence for an agent within the selected trace scope.

use super::{group, Comparison, Occurrence};
use crate::{
    activation::Report,
    conflict_query::{timestamp, Scope, TimeWindow},
    scoped_calls::next_offset,
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
    compare: Option<Selection>,
    group: Option<String>,
    #[serde(default)]
    include_excerpts: bool,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    compare_offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    after: Option<String>,
    before: Option<String>,
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
    let primary = TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?;
    let comparison = request
        .compare
        .as_ref()
        .map(|side| TimeWindow::parse(side.after.as_deref(), side.before.as_deref()))
        .transpose()?;
    let membership: BTreeMap<_, _> = runs
        .into_iter()
        .filter_map(|run| {
            let a = includes(run, &request.scope, request.thread.as_ref());
            let b = request
                .compare
                .as_ref()
                .is_some_and(|side| includes(run, &side.scope, side.thread.as_ref()));
            (a || b).then_some((run.id.as_str(), (run, a, b)))
        })
        .collect();
    let scoped: Vec<_> = membership.values().map(|(run, _, _)| *run).collect();
    let grouped = group(&scoped, reports);
    let groups: Vec<_> = grouped
        .iter()
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
        .filter_map(|entry| {
            let mut sides = [Vec::new(), Vec::new()];
            for item in &entry.occurrences {
                let (_, a, b) = membership[item.location.run_id.as_str()];
                let time = item.started_at.and_then(|value| timestamp(value).ok());
                if a && primary.includes(time) {
                    sides[0].push(item);
                }
                if b && comparison
                    .as_ref()
                    .is_some_and(|window| window.includes(time))
                {
                    sides[1].push(item);
                }
            }
            sides
                .iter()
                .any(|side| !side.is_empty())
                .then_some((&entry.comparison, sides))
        })
        .collect();

    if let Some(key) = &request.group {
        for (identity, [mut a, mut b]) in groups {
            if group_key(identity)? != *key {
                continue;
            }
            let group = group_summary(identity, key, request, [&a, &b]);
            if comparison.is_some() {
                return Ok(json!({
                    "kind": "activation_evidence_comparison", "block": request.block,
                    "captured_excerpts_enabled": allow_excerpts, "group": group,
                    "scope_a": occurrence_page(&mut a, request.block, request.offset, request.limit),
                    "scope_b": occurrence_page(&mut b, request.block, request.compare_offset, request.limit),
                }));
            }
            let mut page = occurrence_page(&mut a, request.block, request.offset, request.limit);
            page["kind"] = json!("activation_evidence");
            page["block"] = json!(request.block);
            page["captured_excerpts_enabled"] = json!(allow_excerpts);
            page["group"] = group;
            return Ok(page);
        }
        return Err("The activation group is unavailable for this scope and block.".into());
    }
    let rows: Vec<_> = groups
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|(identity, [a, b])| {
            Ok(group_summary(
                identity,
                &group_key(identity)?,
                request,
                [a, b],
            ))
        })
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "kind": if comparison.is_some() { "activation_group_comparison" } else { "activation_groups" },
        "block": request.block,
        "captured_excerpts_enabled": allow_excerpts,
        "total_groups": groups.len(),
        "groups": rows,
        "next_offset": next_offset(request.offset, request.limit, groups.len()),
    }))
}

fn includes(run: &Run, scope: &Scope, thread: Option<&Thread>) -> bool {
    scope.includes(run) && thread.is_none_or(|thread| thread.includes(run))
}

fn group_key(identity: &Comparison<'_>) -> Result<String, String> {
    let bytes = serde_json::to_vec(identity).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn group_summary(
    comparison: &Comparison<'_>,
    key: &str,
    request: &Request,
    [a, b]: [&[&Occurrence<'_>]; 2],
) -> Value {
    let mut result = json!({
        "key": key, "model_id": comparison.model_id, "model_revision": comparison.model_revision,
        "runtime": comparison.runtime, "layer_count": comparison.layer_count,
        "decision_kind": comparison.decision_kind, "decision_label": comparison.decision_label,
        "source_id": comparison.source_id, "source_version": comparison.source_version,
        "checkpoint": comparison.checkpoint.label, "output_token": comparison.checkpoint.output_token,
    });
    if request.compare.is_some() {
        result["scope_a"] = json!(summarize(a, request.block));
        result["scope_b"] = json!(summarize(b, request.block));
    } else if let Some(summary) = summarize(a, request.block) {
        result
            .as_object_mut()
            .expect("group object")
            .extend(summary);
    }
    if request.include_excerpts {
        result["contrasts"] = json!({
            "before": turn_evidence::excerpt(comparison.before),
            "after": turn_evidence::excerpt(comparison.after),
            "alternatives": comparison.checkpoint.alternatives.each_ref().map(|text| turn_evidence::excerpt(text)),
        });
    }
    result
}

fn summarize(
    occurrences: &[&Occurrence<'_>],
    block: usize,
) -> Option<serde_json::Map<String, Value>> {
    if occurrences.is_empty() {
        return None;
    }
    let mut decisions = BTreeMap::new();
    let mut conversations = BTreeSet::new();
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for item in occurrences {
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
    Some(
        [
            ("decision_count", json!(decisions.len())),
            ("measurement_count", json!(occurrences.len())),
            ("conversation_count", json!(conversations.len())),
            (
                "lower_score_count",
                json!(decisions.values().filter(|(lower, _)| *lower).count()),
            ),
            (
                "flipped_count",
                json!(decisions.values().filter(|(_, flipped)| *flipped).count()),
            ),
            (
                "change_log_odds",
                json!({"minimum": minimum, "maximum": maximum}),
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect(),
    )
}

fn occurrence_page(
    items: &mut [&Occurrence<'_>],
    block: usize,
    offset: usize,
    limit: usize,
) -> Value {
    items.sort_by_cached_key(|item| {
        let time = item.started_at.and_then(|value| timestamp(value).ok());
        (
            time.is_none(),
            time,
            item.location,
            item.report_id,
            item.context_id,
        )
    });
    json!({
        "total_occurrences": items.len(),
        "occurrences": items.iter().skip(offset).take(limit).map(|item| evidence(item, block)).collect::<Vec<_>>(),
        "next_offset": next_offset(offset, limit, items.len()),
    })
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
        "inspector_path": crate::activation::source_swaps::inspector_path(
            location, item.report_id, "checkpoint", checkpoint, block, item.context_id,
        ),
        "source_inspector_path": format!("{path}&context={context}"),
    })
}
