use super::{ReplayInterval, Summary, TargetStatus};
use crate::{
    scoped_calls::{self, conversation_key, next_offset, ScopedCall, Selection},
    turn_evidence::inspector_path,
    Call, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

/// Select one or two recorded scopes and optionally drill into a group.
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

impl Request {
    pub(super) fn includes_source(&self, source_id: &str) -> bool {
        !source_id.trim().is_empty() && self.source_id.as_deref().is_none_or(|id| id == source_id)
    }
}

fn default_limit() -> usize {
    20
}

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Candidate,
    Shadow,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Shadow => "shadow",
        }
    }
}

pub(crate) fn source_path(
    kind: Kind,
    run: &str,
    call: &str,
    report: &str,
    context: &str,
) -> String {
    let path = inspector_path(run, call, None);
    let report = utf8_percent_encode(report, NON_ALPHANUMERIC);
    let source = utf8_percent_encode(context, NON_ALPHANUMERIC);
    let anchor = match kind {
        Kind::Candidate => "probe",
        Kind::Shadow => "shadow",
    };
    format!("{path}&{anchor}={report}&{anchor}_source={source}")
}

#[derive(Clone, Copy)]
pub(super) enum Measure<'a> {
    Candidate {
        baseline_log_odds: f64,
        edited_log_odds: f64,
        change_log_odds: f64,
    },
    Shadow {
        baseline: &'a Summary,
        edited: &'a Summary,
        difference_pp: Option<f64>,
        difference_95_interval_pp: Option<ReplayInterval>,
    },
}

impl Measure<'_> {
    fn change(self) -> Option<f64> {
        match self {
            Self::Candidate {
                change_log_odds, ..
            } => Some(change_log_odds),
            Self::Shadow { difference_pp, .. } => difference_pp,
        }
    }

    fn reversed(self) -> bool {
        matches!(self, Self::Candidate { baseline_log_odds, edited_log_odds, .. }
            if baseline_log_odds != 0.0 && edited_log_odds != 0.0
                && baseline_log_odds.is_sign_positive() != edited_log_odds.is_sign_positive())
    }

    fn append_fields(self, record: &mut Value) {
        match self {
            Self::Candidate {
                baseline_log_odds,
                edited_log_odds,
                change_log_odds,
            } => {
                record["baseline_log_odds"] = json!(baseline_log_odds);
                record["edited_log_odds"] = json!(edited_log_odds);
                record["change_log_odds"] = json!(change_log_odds);
                record["reversed"] = json!(self.reversed());
            }
            Self::Shadow {
                baseline,
                edited,
                difference_pp,
                difference_95_interval_pp,
            } => {
                record["baseline"] = json!(baseline);
                record["edited"] = json!(edited);
                record["difference_pp"] = json!(difference_pp);
                record["difference_95_interval_pp"] = json!(difference_95_interval_pp);
            }
        }
    }
}

pub(super) struct Observation<'a, Setup> {
    pub key: String,
    pub setup: Setup,
    pub report_id: &'a str,
    pub context_id: &'a str,
    pub measure: Measure<'a>,
}

struct Group<'a, Setup> {
    setup: Setup,
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
    measure: Measure<'a>,
}

impl<'a> Occurrence<'a> {
    fn evidence(self, kind: Kind) -> Value {
        let mut record = json!({
            "run_id": self.run.id, "call_id": self.call.id, "report_id": self.report_id, "context_id": self.context_id,
            "trace_source": self.run.source, "organization_id": self.run.organization_id, "user_id": self.run.user_id,
            "conversation_id": self.run.conversation_id, "turn_id": self.call.turn_id, "started_at": self.call.started_at,
            "inspector_path": source_path(kind, &self.run.id, &self.call.id, self.report_id, self.context_id),
        });
        self.measure.append_fields(&mut record);
        record
    }
}

struct Stats {
    fields: Value,
    measured_conversations: usize,
    magnitude: f64,
}

// Each identified conversation counts once per comparable edit. An unfinished trial
// keeps its conversation incomplete even if another call reached the target.
fn edited_targets(rows: &[Occurrence]) -> BTreeMap<TargetStatus, usize> {
    use TargetStatus::{Incomplete, Met, Missed, Mixed};
    let mut by_conversation = BTreeMap::new();
    for row in rows {
        let (Some(key), Measure::Shadow { edited, .. }) = (conversation_key(row.run), row.measure)
        else {
            continue;
        };
        let status = edited.target_status();
        by_conversation
            .entry(key)
            .and_modify(|prior| {
                if *prior == Incomplete || status == Incomplete {
                    *prior = Incomplete;
                } else if *prior != status {
                    *prior = Mixed;
                }
            })
            .or_insert(status);
    }
    let mut counts = BTreeMap::from([(Met, 0), (Missed, 0), (Mixed, 0), (Incomplete, 0)]);
    for status in by_conversation.into_values() {
        *counts.entry(status).or_default() += 1;
    }
    counts
}

fn summarize(rows: &[Occurrence<'_>], kind: Kind) -> Option<Stats> {
    if rows.is_empty() {
        return None;
    }
    let mut calls = BTreeSet::new();
    let mut conversations = BTreeSet::new();
    let mut measured_conversations = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut reversals = 0;
    let (mut increased, mut decreased, mut unchanged) = (0, 0, 0);
    let mut bounds: Option<(f64, f64)> = None;
    for &item in rows {
        calls.insert((&item.run.id, &item.call.id));
        conversations.extend(conversation_key(item.run));
        organizations.extend(item.run.organization_id.as_deref());
        reversals += usize::from(item.measure.reversed());
        if let Some(change) = item.measure.change() {
            measured_conversations.extend(conversation_key(item.run));
            increased += usize::from(change > 0.0);
            decreased += usize::from(change < 0.0);
            unchanged += usize::from(change == 0.0);
            bounds = Some(bounds.map_or((change, change), |(min, max)| {
                (min.min(change), max.max(change))
            }));
        }
    }
    let mut fields = json!({
        "measurements": rows.len(), "calls": calls.len(),
        "conversations": conversations.len(), "organizations": organizations.len(),
    });
    match kind {
        Kind::Candidate => {
            fields["reversals"] = json!(reversals);
            fields["min_change_log_odds"] = json!(bounds.map(|(min, _)| min));
            fields["max_change_log_odds"] = json!(bounds.map(|(_, max)| max));
        }
        Kind::Shadow => {
            fields["edited_target_conversations"] = json!(edited_targets(rows));
            fields["measured_conversations"] = json!(measured_conversations.len());
            fields["increased"] = json!(increased);
            fields["decreased"] = json!(decreased);
            fields["unchanged"] = json!(unchanged);
            fields["unavailable"] = json!(rows.len() - increased - decreased - unchanged);
            fields["min_difference_pp"] = json!(bounds.map(|(min, _)| min));
            fields["max_difference_pp"] = json!(bounds.map(|(_, max)| max));
        }
    }
    Some(Stats {
        fields,
        measured_conversations: measured_conversations.len(),
        magnitude: bounds.map_or(0.0, |(min, max)| min.abs().max(max.abs())),
    })
}

struct Row<'a, Setup> {
    key: &'a str,
    setup: &'a Setup,
    primary: Option<Stats>,
    comparison: Option<Stats>,
}

impl<Setup: Serialize> Row<'_, Setup> {
    fn rank(&self) -> (usize, f64) {
        self.primary
            .iter()
            .chain(&self.comparison)
            .fold((0, 0.0), |(count, magnitude), stats| {
                (
                    count.max(stats.measured_conversations),
                    magnitude.max(stats.magnitude),
                )
            })
    }

    fn value(&self) -> Value {
        json!({
            "key": self.key, "setup": self.setup,
            "primary": self.primary.as_ref().map(|stats| &stats.fields),
            "comparison": self.comparison.as_ref().map(|stats| &stats.fields),
        })
    }
}

pub(super) fn query<'a, Setup: Serialize>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    kind: Kind,
    observe: impl Fn(&'a Run, &'a Call) -> Result<Vec<Observation<'a, Setup>>, String>,
) -> Result<Value, String> {
    if request.version != 1 {
        return Err(format!(
            "{} pattern queries require version 1.",
            kind.name()
        ));
    }
    if !(1..=50).contains(&request.limit) {
        return Err("Choose a page size from 1 to 50.".into());
    }
    let mut groups = BTreeMap::new();
    scoped_calls::visit(
        &request.selection,
        request.compare.as_ref(),
        runs,
        |ScopedCall {
             run,
             call,
             time,
             primary,
             comparison,
         }| {
            for item in observe(run, call)? {
                if request
                    .group
                    .as_ref()
                    .is_some_and(|group| group != &item.key)
                {
                    continue;
                }
                let group = groups.entry(item.key).or_insert_with(|| Group {
                    setup: item.setup,
                    primary: Vec::new(),
                    comparison: Vec::new(),
                });
                let occurrence = Occurrence {
                    run,
                    call,
                    time,
                    report_id: item.report_id,
                    context_id: item.context_id,
                    measure: item.measure,
                };
                if primary {
                    group.primary.push(occurrence);
                }
                if comparison {
                    group.comparison.push(occurrence);
                }
            }
            Ok(())
        },
    )?;

    let rows = groups
        .iter()
        .map(|(key, group)| Row {
            key,
            setup: &group.setup,
            primary: summarize(&group.primary, kind),
            comparison: summarize(&group.comparison, kind),
        })
        .filter(|row| row.rank().0 >= request.min_conversations);
    let compared = request.compare.is_some();
    if let Some(key) = &request.group {
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| format!("The {} group is unavailable in this scope.", kind.name()))?
            .value();
        let group = groups.get_mut(key).expect("selected group");
        return Ok(json!({
            "kind": format!("{}_pattern_evidence", kind.name()), "group": row, "compared": compared,
            "primary": occurrence_page(&mut group.primary, request.offset, request.limit, kind),
            "comparison": compared.then(|| occurrence_page(&mut group.comparison, request.compare_offset, request.limit, kind)),
        }));
    }
    let mut rows: Vec<_> = rows.collect();
    rows.sort_by(|left, right| {
        let (left_count, left_effect) = left.rank();
        let (right_count, right_effect) = right.rank();
        right_count
            .cmp(&left_count)
            .then_with(|| right_effect.total_cmp(&left_effect))
            .then_with(|| left.key.cmp(right.key))
    });
    Ok(json!({
        "kind": format!("{}_pattern_groups", kind.name()), "total_groups": rows.len(), "compared": compared,
        "groups": rows.iter().skip(request.offset).take(request.limit).map(Row::value).collect::<Vec<_>>(),
        "next_offset": next_offset(request.offset, request.limit, rows.len()),
    }))
}

fn occurrence_page(items: &mut [Occurrence<'_>], offset: usize, limit: usize, kind: Kind) -> Value {
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
        "rows": items.iter().skip(offset).take(limit).map(|item| item.evidence(kind)).collect::<Vec<_>>(),
        "next_offset": next_offset(offset, limit, items.len()),
    })
}
