//! Find recurring preference shifts under comparable candidate probes.

use super::{recorded, sha256, Measurement, RecordedEffect, Verification};
use crate::{
    activation::patterns::query::Thread,
    conflict_query::{timestamp, Scope, TimeWindow},
    replay::Operation,
    turn_evidence::inspector_path,
    Call, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    #[serde(default)]
    selection: Selection,
    compare: Option<Selection>,
    source_id: Option<String>,
    #[serde(default)]
    min_conversations: usize,
    group: Option<String>,
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    after: Option<String>,
    before: Option<String>,
}

impl Selection {
    fn includes(&self, run: &Run) -> bool {
        self.scope.includes(run)
            && self
                .thread
                .as_ref()
                .is_none_or(|thread| thread.includes(run))
    }

    fn window(&self) -> Result<TimeWindow, String> {
        TimeWindow::parse(self.after.as_deref(), self.before.as_deref())
    }
}

#[derive(Serialize)]
struct Setup<'a> {
    source_id: &'a str,
    source_version: Option<&'a str>,
    scorer_id: &'a str,
    model_id: &'a str,
    model_revision: &'a str,
    runtime: &'a str,
    candidate_ids: [&'a str; 2],
    operation: Operation,
}

struct Group<'a> {
    setup: Setup<'a>,
    primary: Vec<Occurrence<'a>>,
    comparison: Vec<Occurrence<'a>>,
}

#[derive(Clone, Copy)]
struct Occurrence<'a> {
    run: &'a Run,
    call: &'a Call,
    report_id: &'a str,
    context_id: &'a str,
    time: Option<OffsetDateTime>,
    baseline_log_odds: f64,
    edited_log_odds: f64,
    change_log_odds: f64,
}

impl Occurrence<'_> {
    fn reversed(self) -> bool {
        self.baseline_log_odds != 0.0
            && self.edited_log_odds != 0.0
            && self.baseline_log_odds.is_sign_positive() != self.edited_log_odds.is_sign_positive()
    }

    fn evidence(self) -> Value {
        let path = inspector_path(&self.run.id, &self.call.id, None);
        let report = utf8_percent_encode(self.report_id, NON_ALPHANUMERIC);
        let source = utf8_percent_encode(self.context_id, NON_ALPHANUMERIC);
        json!({
            "run_id": self.run.id, "call_id": self.call.id, "report_id": self.report_id, "context_id": self.context_id,
            "trace_source": self.run.source, "organization_id": self.run.organization_id, "user_id": self.run.user_id,
            "conversation_id": self.run.conversation_id, "turn_id": self.call.turn_id, "started_at": self.call.started_at,
            "baseline_log_odds": self.baseline_log_odds, "edited_log_odds": self.edited_log_odds,
            "change_log_odds": self.change_log_odds, "reversed": self.reversed(),
            "inspector_path": format!("{path}&probe={report}&probe_source={source}"),
        })
    }
}

#[derive(Serialize)]
struct Summary {
    measurements: usize,
    calls: usize,
    conversations: usize,
    organizations: usize,
    reversals: usize,
    min_change_log_odds: f64,
    max_change_log_odds: f64,
}

fn summarize(rows: &[Occurrence<'_>]) -> Option<Summary> {
    let first = rows.first()?;
    let mut calls = BTreeSet::new();
    let mut conversations = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut reversals = 0;
    let mut min = first.change_log_odds;
    let mut max = first.change_log_odds;
    for &item in rows {
        let run = item.run;
        calls.insert((&run.id, &item.call.id));
        if let Some(conversation) = &run.conversation_id {
            conversations.insert((
                &run.source,
                &run.organization_id,
                &run.user_id,
                conversation,
            ));
        }
        organizations.extend(run.organization_id.as_deref());
        reversals += usize::from(item.reversed());
        min = min.min(item.change_log_odds);
        max = max.max(item.change_log_odds);
    }
    Some(Summary {
        measurements: rows.len(),
        calls: calls.len(),
        conversations: conversations.len(),
        organizations: organizations.len(),
        reversals,
        min_change_log_odds: min,
        max_change_log_odds: max,
    })
}

fn setup<'a>(
    measurement: &Measurement<'a>,
    effect: &RecordedEffect<'a>,
    source_id: &'a str,
) -> Result<(String, Setup<'a>), String> {
    let setup = Setup {
        source_id,
        source_version: effect.source_version,
        scorer_id: measurement.scorer_id,
        model_id: measurement.model_id,
        model_revision: measurement.model_revision,
        runtime: measurement.runtime,
        candidate_ids: measurement
            .candidates
            .each_ref()
            .map(|candidate| candidate.id.as_str()),
        operation: effect.operation,
    };
    let identity = serde_json::to_vec(&(
        "nexusweave.candidate_pattern.v1",
        &setup,
        measurement.candidates,
        measurement.assistant_prefix,
        &measurement.scores.tokenizer_sha256,
        &measurement.scores.assistant_prefix_token_ids,
        &measurement.scores.candidate_token_ids,
        &effect.context.kind,
        &effect.context.role,
        &effect.captured_context_sha256,
        effect.replacement,
    ))
    .map_err(|error| error.to_string())?;
    Ok((sha256(&identity), setup))
}

#[derive(Serialize)]
struct Row<'a> {
    key: &'a str,
    setup: &'a Setup<'a>,
    primary: Option<Summary>,
    comparison: Option<Summary>,
}

impl Row<'_> {
    fn rank(&self) -> (usize, f64) {
        self.primary.iter().chain(self.comparison.iter()).fold(
            (0, 0.0),
            |(count, effect), stats| {
                (
                    count.max(stats.conversations),
                    effect
                        .max(stats.min_change_log_odds.abs())
                        .max(stats.max_change_log_odds.abs()),
                )
            },
        )
    }
}

/// Query verified probe measurements within one or two trace scopes.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
) -> Result<Value, String> {
    if request.version != 1 {
        return Err("Candidate pattern queries require version 1.".into());
    }
    if !(1..=50).contains(&request.limit) {
        return Err("Choose a page size from 1 to 50.".into());
    }
    let primary_window = request.selection.window()?;
    let comparison_window = request
        .compare
        .as_ref()
        .map(Selection::window)
        .transpose()?;
    let mut groups = BTreeMap::new();
    for run in runs {
        let primary_run = request.selection.includes(run);
        let compare_run = request
            .compare
            .as_ref()
            .is_some_and(|selection| selection.includes(run));
        if !primary_run && !compare_run {
            continue;
        }
        for call in &run.calls {
            let time = call
                .started_at
                .as_deref()
                .and_then(|value| timestamp(value).ok());
            let primary = primary_run && primary_window.includes(time);
            let comparison = compare_run
                && comparison_window
                    .as_ref()
                    .is_some_and(|window| window.includes(time));
            if !primary && !comparison {
                continue;
            }
            for probe in recorded(&run.id, call) {
                let Verification::Matched { measurement } = probe.verification else {
                    continue;
                };
                for effect in &measurement.effects {
                    let Some(source_id) = effect.source_id.filter(|id| !id.trim().is_empty())
                    else {
                        continue;
                    };
                    if request
                        .source_id
                        .as_deref()
                        .is_some_and(|id| id != source_id)
                    {
                        continue;
                    }
                    let (key, setup) = setup(&measurement, effect, source_id)?;
                    if request.group.as_ref().is_some_and(|group| group != &key) {
                        continue;
                    }
                    let group = groups.entry(key).or_insert_with(|| Group {
                        setup,
                        primary: Vec::new(),
                        comparison: Vec::new(),
                    });
                    let occurrence = Occurrence {
                        run,
                        call,
                        report_id: probe.report_id.expect("matched probe ID"),
                        context_id: effect.context_id,
                        time,
                        baseline_log_odds: measurement.baseline_log_odds,
                        edited_log_odds: effect.log_odds,
                        change_log_odds: effect.change_log_odds,
                    };
                    if primary {
                        group.primary.push(occurrence);
                    }
                    if comparison {
                        group.comparison.push(occurrence);
                    }
                }
            }
        }
    }

    let mut rows: Vec<_> = groups
        .iter()
        .map(|(key, group)| Row {
            key,
            setup: &group.setup,
            primary: summarize(&group.primary),
            comparison: summarize(&group.comparison),
        })
        .filter(|row| row.rank().0 >= request.min_conversations)
        .collect();
    if let Some(key) = &request.group {
        let row = serde_json::to_value(
            rows.first()
                .ok_or("The candidate group is unavailable in this scope.")?,
        )
        .map_err(|error| error.to_string())?;
        let group = groups.get_mut(key).expect("selected group");
        return Ok(json!({
            "kind": "candidate_pattern_evidence", "group": row, "compared": request.compare.is_some(),
            "primary": occurrence_page(&mut group.primary, request.offset, request.limit),
            "comparison": request.compare.as_ref().map(|_| occurrence_page(&mut group.comparison, request.compare_offset, request.limit)),
        }));
    }
    rows.sort_by(|left, right| {
        let (left_count, left_effect) = left.rank();
        let (right_count, right_effect) = right.rank();
        right_count
            .cmp(&left_count)
            .then_with(|| right_effect.total_cmp(&left_effect))
            .then_with(|| left.key.cmp(right.key))
    });
    Ok(json!({
        "kind": "candidate_pattern_groups", "total_groups": rows.len(), "compared": request.compare.is_some(),
        "groups": rows.iter().skip(request.offset).take(request.limit).collect::<Vec<_>>(),
        "next_offset": next_offset(request.offset, request.limit, rows.len()),
    }))
}

fn next_offset(offset: usize, limit: usize, count: usize) -> Option<usize> {
    let next = offset.saturating_add(limit);
    (next < count).then_some(next)
}

fn occurrence_page(items: &mut [Occurrence<'_>], offset: usize, limit: usize) -> Value {
    items.sort_by_key(|item| {
        (
            item.time.is_none(),
            item.time,
            &item.run.id,
            &item.call.id,
            item.report_id,
            item.context_id,
        )
    });
    json!({
        "total": items.len(),
        "rows": items.iter().skip(offset).take(limit).map(|item| item.evidence()).collect::<Vec<_>>(),
        "next_offset": next_offset(offset, limit, items.len()),
    })
}
