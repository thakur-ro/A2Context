//! Query comparable controlled replays in one or two recorded scopes.

use super::{
    group_reports, Comparison, ComparisonTarget, ContextChange, EffectOccurrence, OccurrenceTarget,
};
use crate::{
    activation::decision_path,
    replay::{Location, MatchRule, Replay},
    scoped_calls::{self, next_offset, Selection},
    turn_evidence::excerpt,
    Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

/// Read a page of groups or contributing decisions within the supplied selections.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    #[serde(default)]
    selection: Selection,
    compare: Option<Selection>,
    source_id: Option<String>,
    kind: Option<Kind>,
    #[serde(default)]
    min_conversations: usize,
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

fn default_limit() -> usize {
    20
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Single,
    Pair,
    Control,
}

fn edits<'a, 'b>(target: &'b ComparisonTarget<'a>) -> (Kind, Vec<&'b ContextChange<'a>>) {
    match target {
        ComparisonTarget::Single(change) => (Kind::Single, vec![change]),
        ComparisonTarget::Pair { pair } => (Kind::Pair, pair.iter().collect()),
        ComparisonTarget::Control {
            target_edit,
            control_edit,
        } => (Kind::Control, vec![target_edit, control_edit]),
    }
}

#[derive(Clone, Copy)]
struct CallEvidence {
    time: Option<OffsetDateTime>,
    baseline_verified: bool,
    included: [bool; 2],
}

#[derive(Clone, Copy)]
struct Measurement<'a, 'b> {
    item: &'b EffectOccurrence<'a>,
    call: CallEvidence,
}

/// Query previously validated replay reports. Pass only runs the caller may read.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    reports: &'a [Replay],
    allow_excerpts: bool,
) -> Result<Value, String> {
    if request.version != 1 || !(1..=50).contains(&request.limit) {
        return Err("Use replay effect query version 1 and a page size from 1 to 50.".into());
    }
    if request.include_excerpts && !allow_excerpts {
        return Err("This MCP process has not enabled excerpts.".into());
    }
    let mut scoped = BTreeMap::new();
    let mut calls = BTreeMap::new();
    scoped_calls::visit(&request.selection, request.compare.as_ref(), runs, |item| {
        scoped.insert(item.run.id.as_str(), item.run);
        calls.insert(
            (item.run.id.as_str(), item.call.id.as_str()),
            CallEvidence {
                time: item.time,
                baseline_verified: item.call.recorded_request.is_some(),
                included: [item.primary, item.comparison],
            },
        );
        Ok(())
    })?;
    let selected = reports
        .iter()
        .filter(|report| calls.contains_key(&call_key(&report.location)));
    let grouped = group_reports(&scoped, selected)?;
    let compared = request.compare.is_some();
    let mut groups = Vec::new();
    let mut total_groups = 0;
    for group in &grouped {
        let (kind, changes) = edits(&group.comparison.target);
        if request.kind.is_some_and(|requested| requested != kind)
            || request
                .source_id
                .as_deref()
                .is_some_and(|source| !changes.iter().any(|edit| edit.source_id == Some(source)))
        {
            continue;
        }
        let bytes = serde_json::to_vec(&group.comparison).map_err(|error| error.to_string())?;
        let key = format!("{:x}", Sha256::digest(bytes));
        if request.group.as_ref().is_some_and(|wanted| wanted != &key) {
            continue;
        }
        let mut sides = [Vec::new(), Vec::new()];
        for item in &group.occurrences {
            let call = calls[&call_key(item.location)];
            for (side, included) in sides.iter_mut().zip(call.included) {
                if included {
                    side.push(Measurement { item, call });
                }
            }
        }
        let [primary, comparison] = sides.each_ref().map(|side| summarize(side));
        if primary
            .iter()
            .chain(&comparison)
            .all(|stats| stats.measured_conversations < request.min_conversations)
        {
            continue;
        }
        if request.group.is_some()
            || (total_groups >= request.offset && groups.len() < request.limit)
        {
            let row = json!({
                "key": key, "setup": setup(&group.comparison, kind, &changes, request.include_excerpts),
                "primary": primary, "comparison": comparison,
            });
            if request.group.is_some() {
                let [mut a, mut b] = sides;
                return Ok(json!({
                    "version": 1, "kind": "replay_effect_evidence", "compared": compared,
                    "captured_excerpts_enabled": allow_excerpts, "group": row,
                    "primary": page(&mut a, request.offset, request.limit),
                    "comparison": compared.then(|| page(&mut b, request.compare_offset, request.limit)),
                }));
            }
            groups.push(row);
        }
        total_groups += 1;
    }
    if request.group.is_some() {
        return Err("The replay effect group is unavailable for this selection.".into());
    }
    Ok(json!({
        "version": 1, "kind": "replay_effect_groups", "compared": compared,
        "captured_excerpts_enabled": allow_excerpts, "total_groups": total_groups, "groups": groups,
        "next_offset": next_offset(request.offset, request.limit, total_groups),
    }))
}

fn call_key(location: &Location) -> (&str, &str) {
    (&location.run_id, &location.call_id)
}

fn setup(
    comparison: &Comparison<'_>,
    kind: Kind,
    changes: &[&ContextChange<'_>],
    include_excerpts: bool,
) -> Value {
    let rule = match &comparison.match_rule {
        MatchRule::Exact => "exact",
        MatchRule::KindAndLabel => "kind_and_label",
        MatchRule::OutcomeFields { .. } => "outcome_fields",
        MatchRule::TextCapture { .. } => "text_capture",
    };
    let edits: Vec<_> = changes
        .iter()
        .map(|change| {
            let mut edit = json!({
                "source_id": change.source_id, "source_version": change.source_version,
                "context_kind": change.context_kind, "context_role": change.context_role,
                "operation": change.operation,
            });
            if include_excerpts {
                edit["replacement_excerpt"] = json!(excerpt(change.replacement));
            }
            edit
        })
        .collect();
    json!({
        "kind": kind, "edits": edits, "runner_id": comparison.runner_id, "model": comparison.model,
        "decision_kind": comparison.decision_kind, "decision_label": comparison.decision_label,
        "match_rule": rule, "sampling_design": comparison.sampling_design,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Lower,
    Same,
    Higher,
    Mixed,
}

#[derive(Default, Serialize)]
struct Summary {
    measurements: usize,
    verified_baselines: usize,
    decisions: usize,
    conversations: usize,
    measured_conversations: usize,
    lower: usize,
    unchanged: usize,
    higher: usize,
    mixed: usize,
    unavailable: usize,
    min_difference_pp: Option<f64>,
    max_difference_pp: Option<f64>,
}

fn summarize(rows: &[Measurement<'_, '_>]) -> Option<Summary> {
    if rows.is_empty() {
        return None;
    }
    let mut result = Summary {
        measurements: rows.len(),
        ..Summary::default()
    };
    let mut decisions = BTreeMap::new();
    let mut conversations = BTreeSet::new();
    let mut measured_conversations = BTreeSet::new();
    for row in rows {
        let item = row.item;
        result.verified_baselines += usize::from(row.call.baseline_verified);
        let conversation = item
            .conversation_id
            .map(|id| (item.trace_source, item.organization_id, item.user_id, id));
        conversations.extend(conversation);
        let direction = decisions.entry(item.location).or_insert(None);
        if let Some(effect) = item.difference_pp {
            measured_conversations.extend(conversation);
            result.min_difference_pp = Some(
                result
                    .min_difference_pp
                    .map_or(effect, |min| min.min(effect)),
            );
            result.max_difference_pp = Some(
                result
                    .max_difference_pp
                    .map_or(effect, |max| max.max(effect)),
            );
            let next = if effect < 0.0 {
                Direction::Lower
            } else if effect > 0.0 {
                Direction::Higher
            } else {
                Direction::Same
            };
            *direction = Some(if direction.is_some_and(|prior| prior != next) {
                Direction::Mixed
            } else {
                next
            });
        }
    }
    result.decisions = decisions.len();
    result.conversations = conversations.len();
    result.measured_conversations = measured_conversations.len();
    for direction in decisions.into_values() {
        match direction {
            Some(Direction::Lower) => result.lower += 1,
            Some(Direction::Same) => result.unchanged += 1,
            Some(Direction::Higher) => result.higher += 1,
            Some(Direction::Mixed) => result.mixed += 1,
            None => result.unavailable += 1,
        }
    }
    Some(result)
}

fn page(rows: &mut [Measurement<'_, '_>], offset: usize, limit: usize) -> Value {
    rows.sort_by_key(|row| {
        (
            row.call.time.is_none(),
            row.call.time,
            row.item.location,
            row.item.replay_id,
        )
    });
    let evidence: Vec<_> = rows
        .iter()
        .skip(offset)
        .take(limit)
        .map(|row| {
            let mut evidence = json!(row.item);
            evidence["baseline_verified"] = json!(row.call.baseline_verified);
            evidence["inspector_path"] = json!(inspector_path(row.item));
            evidence
        })
        .collect();
    json!({"total_measurements": rows.len(), "rows": evidence, "next_offset": next_offset(offset, limit, rows.len())})
}

fn inspector_path(item: &EffectOccurrence<'_>) -> String {
    let decision = decision_path(item.location);
    let report = utf8_percent_encode(item.replay_id, NON_ALPHANUMERIC);
    let base = format!("{decision}&replay={report}");
    match &item.target {
        OccurrenceTarget::Single { context_id } => format!(
            "{base}&variant={}",
            utf8_percent_encode(context_id, NON_ALPHANUMERIC)
        ),
        OccurrenceTarget::Pair { .. } => base,
        OccurrenceTarget::Control {
            control_context_id, ..
        } => format!(
            "{base}&control={}",
            utf8_percent_encode(control_context_id, NON_ALPHANUMERIC)
        ),
    }
}
