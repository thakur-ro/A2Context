//! Page through named sources and their captured occurrences.

use super::{group, SourceOccurrence, SourceProfile};
use crate::{
    activation::{
        self,
        patterns::query::Thread,
        visibility::query::{measured_masks, SourceMask},
    },
    conflict_query::{timestamp, Scope, TimeWindow},
    context_links, replay, Availability, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    after: Option<String>,
    before: Option<String>,
    source_id: Option<String>,
    source_version: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

type Input<'a> = (&'a str, &'a str, &'a str);

#[derive(Default)]
struct Measurements<'a> {
    replay_count: usize,
    mask_count: usize,
    replay: Option<(&'a replay::Replay, &'a replay::Variant)>,
    mask: Option<SourceMask<'a>>,
}

#[derive(Serialize)]
struct Counts<'a> {
    source_id: &'a str,
    occurrence_count: usize,
    call_count: usize,
    conversation_count: usize,
    user_count: usize,
    organization_count: usize,
    linked_decision_count: usize,
    version_count: usize,
    unversioned_occurrences: usize,
    included_occurrences: usize,
    replayed_occurrences: usize,
    masked_occurrences: usize,
}

/// Read the runs and reports already validated and made available to the caller.
pub fn query<'a>(
    request: &Request,
    runs: impl Iterator<Item = &'a Run>,
    replays: &[replay::Replay],
    reports: &[activation::Report],
) -> Result<Value, String> {
    if request.version != 1 || !(1..=50).contains(&request.limit) {
        return Err("Use context source query version 1 and a page size from 1 to 50.".into());
    }
    let window = TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?;
    let runs: Vec<_> = runs
        .filter(|run| {
            request.scope.includes(run)
                && request
                    .thread
                    .as_ref()
                    .is_none_or(|thread| thread.includes(run))
        })
        .collect();
    let links: Vec<_> = runs
        .iter()
        .flat_map(|run| context_links::link_run(run))
        .collect();
    let mut profiles = group(runs.into_iter(), &links);
    profiles.retain_mut(|profile| {
        if request
            .source_id
            .as_deref()
            .is_some_and(|id| profile.source_id != id)
        {
            return false;
        }
        profile.occurrences.retain(|row| {
            window.includes(started_at(row))
                && request
                    .source_version
                    .as_deref()
                    .is_none_or(|version| row.source_version == Some(version))
        });
        !profile.occurrences.is_empty()
    });
    let eligible = profiles
        .iter()
        .flat_map(|profile| profile.occurrences.iter().map(input_key))
        .collect();
    let measurements = index_measurements(&eligible, replays, reports);
    if request.source_id.is_some() {
        return Ok(occurrences(request, profiles.pop(), &measurements));
    }
    let mut counts: Vec<_> = profiles
        .iter()
        .map(|profile| summarize(profile, &measurements))
        .collect();
    counts.sort_by_key(|row| (Reverse(row.call_count), row.source_id));
    let rows: Vec<_> = counts
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .collect();
    Ok(json!({
        "version": 1, "kind": "context_source_profiles", "total_sources": counts.len(),
        "rows": rows, "next_offset": next_offset(request.offset, rows.len(), counts.len()),
    }))
}

fn input_key<'a>(row: &SourceOccurrence<'a>) -> Input<'a> {
    (row.run_id, row.call_id, row.context_id)
}

fn started_at(row: &SourceOccurrence<'_>) -> Option<OffsetDateTime> {
    row.started_at.and_then(|value| timestamp(value).ok())
}

fn index_measurements<'a>(
    eligible: &BTreeSet<Input<'_>>,
    replays: &'a [replay::Replay],
    reports: &'a [activation::Report],
) -> BTreeMap<Input<'a>, Measurements<'a>> {
    let mut index = BTreeMap::<_, Measurements<'_>>::new();
    for report in replays {
        for variant in &report.variants {
            let key = (
                report.location.run_id.as_str(),
                report.location.call_id.as_str(),
                variant.intervention.context_id.as_str(),
            );
            if variant.difference_pp.is_some() && eligible.contains(&key) {
                let entry = index.entry(key).or_default();
                entry.replay_count += 1;
                entry.replay.get_or_insert((report, variant));
            }
        }
    }
    for mask in measured_masks(reports) {
        let (location, id) = mask.input();
        let key = (location.run_id.as_str(), location.call_id.as_str(), id);
        if eligible.contains(&key) {
            let entry = index.entry(key).or_default();
            entry.mask_count += 1;
            entry.mask.get_or_insert(mask);
        }
    }
    index
}

fn summarize<'a>(
    profile: &SourceProfile<'a>,
    measurements: &BTreeMap<Input<'_>, Measurements<'_>>,
) -> Counts<'a> {
    let mut calls = BTreeSet::new();
    let mut threads = BTreeSet::new();
    let mut users = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut decisions = BTreeSet::new();
    let mut versions = BTreeSet::new();
    let (mut unversioned, mut included, mut replays, mut masks) = (0, 0, 0, 0);
    for row in &profile.occurrences {
        calls.insert((row.run_id, row.call_id));
        if let Some(thread) = row.conversation_id {
            threads.insert((row.trace_source, row.organization_id, row.user_id, thread));
        }
        if let Some(user) = row.user_id {
            users.insert((row.organization_id, user));
        }
        if let Some(org) = row.organization_id {
            organizations.insert(org);
        }
        for id in &row.linked_decision_ids {
            decisions.insert((row.run_id, row.call_id, id));
        }
        if let Some(version) = row.source_version {
            versions.insert(version);
        } else {
            unversioned += 1;
        }
        included += usize::from(row.availability == Availability::Included);
        if let Some(entry) = measurements.get(&input_key(row)) {
            replays += usize::from(entry.replay_count > 0);
            masks += usize::from(entry.mask_count > 0);
        }
    }
    Counts {
        source_id: profile.source_id,
        occurrence_count: profile.occurrences.len(),
        call_count: calls.len(),
        conversation_count: threads.len(),
        user_count: users.len(),
        organization_count: organizations.len(),
        linked_decision_count: decisions.len(),
        version_count: versions.len(),
        unversioned_occurrences: unversioned,
        included_occurrences: included,
        replayed_occurrences: replays,
        masked_occurrences: masks,
    }
}

fn occurrences(
    request: &Request,
    profile: Option<SourceProfile<'_>>,
    measurements: &BTreeMap<Input<'_>, Measurements<'_>>,
) -> Value {
    let totals = profile
        .as_ref()
        .map(|profile| summarize(profile, measurements));
    let mut items = profile
        .map(|profile| profile.occurrences)
        .unwrap_or_default();
    items.sort_by_cached_key(|row| {
        let time = started_at(row);
        (
            time.is_none(),
            time,
            row.run_id,
            row.call_id,
            row.context_id,
        )
    });
    let rows: Vec<_> = items
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|row| detail(row, measurements.get(&input_key(row))))
        .collect();
    json!({
        "version": 1, "kind": "context_source_occurrences", "source": totals,
        "total_occurrences": items.len(), "rows": rows,
        "next_offset": next_offset(request.offset, rows.len(), items.len()),
    })
}

fn detail(row: &SourceOccurrence<'_>, measurements: Option<&Measurements<'_>>) -> Value {
    let mut value = json!(row);
    let context = encode(row.context_id);
    let path = activation::decision_view_path(
        row.run_id,
        row.call_id,
        row.linked_decision_ids.first().copied(),
    );
    value["inspector_path"] = json!(format!("{path}&context={context}"));
    value["replay_measurement_count"] = json!(measurements.map_or(0, |entry| entry.replay_count));
    value["mask_measurement_count"] = json!(measurements.map_or(0, |entry| entry.mask_count));
    if let Some((report, variant)) = measurements.and_then(|entry| entry.replay) {
        let path = activation::decision_path(&report.location);
        let replay = encode(&report.id);
        value["replay_example"] = json!({
            "replay_id": report.id, "location": report.location, "difference_pp": variant.difference_pp,
            "inspector_path": format!("{path}&replay={replay}&variant={context}"),
        });
    }
    if let Some(mask) = measurements.and_then(|entry| entry.mask.as_ref()) {
        value["mask_example"] = mask.reference();
    }
    value
}

fn encode(value: &str) -> impl std::fmt::Display + '_ {
    utf8_percent_encode(value, NON_ALPHANUMERIC)
}

fn next_offset(offset: usize, shown: usize, total: usize) -> Option<usize> {
    let next = offset.saturating_add(shown);
    (next < total).then_some(next)
}
