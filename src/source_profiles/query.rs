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
    search: Option<String>,
    compare: Option<Selection>,
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

type Input<'a> = (&'a str, &'a str, &'a str);
type Profiles<'p, 'a> = BTreeMap<&'a str, Vec<&'p SourceOccurrence<'a>>>;

struct Cohort<'a> {
    runs: BTreeSet<&'a str>,
    window: TimeWindow,
}

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
    versions: Vec<VersionCounts<'a>>,
}

#[derive(Serialize)]
struct VersionCounts<'a> {
    source_version: Option<&'a str>,
    included_occurrences: usize,
    unavailable_occurrences: usize,
    captured_forms: usize,
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
    let runs: Vec<_> = runs.collect();
    let primary = Cohort {
        runs: scoped_run_ids(&runs, &request.scope, request.thread.as_ref()),
        window: TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?,
    };
    let secondary = request
        .compare
        .as_ref()
        .map(|compare| {
            Ok::<_, String>(Cohort {
                runs: scoped_run_ids(&runs, &compare.scope, compare.thread.as_ref()),
                window: TimeWindow::parse(compare.after.as_deref(), compare.before.as_deref())?,
            })
        })
        .transpose()?;
    let runs: Vec<_> = runs
        .into_iter()
        .filter(|run| {
            primary.runs.contains(run.id.as_str())
                || secondary
                    .as_ref()
                    .is_some_and(|cohort| cohort.runs.contains(run.id.as_str()))
        })
        .collect();
    let links: Vec<_> = runs
        .iter()
        .flat_map(|run| context_links::link_run(run))
        .collect();
    let profiles = group(runs.into_iter(), &links);
    let search = request
        .search
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let mut left = select_profiles(&profiles, &primary, request);
    let mut right = secondary
        .as_ref()
        .map(|cohort| select_profiles(&profiles, cohort, request));
    if !search.is_empty() {
        let matched: BTreeSet<_> = left
            .iter()
            .chain(right.iter().flat_map(BTreeMap::iter))
            .filter(|(id, rows)| matches_search(id, rows, &search))
            .map(|(id, _)| *id)
            .collect();
        left.retain(|id, _| matched.contains(id));
        if let Some(right) = &mut right {
            right.retain(|id, _| matched.contains(id));
        }
    }
    let eligible = left
        .values()
        .chain(right.iter().flat_map(BTreeMap::values))
        .flatten()
        .map(|row| input_key(row))
        .collect();
    let measurements = index_measurements(&eligible, replays, reports);
    if let Some(id) = request.source_id.as_deref() {
        let left = left
            .get(id)
            .map(|rows| occurrences(id, rows, request.offset, request.limit, &measurements));
        if let Some(right) = right {
            let right = right.get(id).map(|rows| {
                occurrences(
                    id,
                    rows,
                    request.compare_offset,
                    request.limit,
                    &measurements,
                )
            });
            return Ok(json!({
                "version": 1, "kind": "context_source_occurrence_comparison", "source_id": id,
                "scope_a": left, "scope_b": right,
            }));
        }
        return Ok(left.unwrap_or_else(|| {
            occurrences(id, &[], request.offset, request.limit, &measurements)
        }));
    }
    if let Some(right) = right {
        return Ok(compare_profiles(request, left, right, &measurements));
    }
    let mut counts: Vec<_> = left
        .iter()
        .map(|(id, rows)| summarize(id, rows, &measurements))
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

fn scoped_run_ids<'a>(
    runs: &[&'a Run],
    scope: &Scope,
    thread: Option<&Thread>,
) -> BTreeSet<&'a str> {
    runs.iter()
        .filter(|run| scope.includes(run) && thread.is_none_or(|thread| thread.includes(run)))
        .map(|run| run.id.as_str())
        .collect()
}

fn select_profiles<'p, 'a>(
    profiles: &'p [SourceProfile<'a>],
    cohort: &Cohort<'_>,
    request: &Request,
) -> Profiles<'p, 'a> {
    profiles
        .iter()
        .filter(|profile| {
            request
                .source_id
                .as_deref()
                .is_none_or(|id| profile.source_id == id)
        })
        .filter_map(|profile| {
            let rows: Vec<_> = profile
                .occurrences
                .iter()
                .filter(|row| {
                    cohort.runs.contains(row.run_id)
                        && cohort.window.includes(started_at(row))
                        && request
                            .source_version
                            .as_deref()
                            .is_none_or(|version| row.source_version == Some(version))
                })
                .collect();
            (!rows.is_empty()).then_some((profile.source_id, rows))
        })
        .collect()
}

fn matches_search(id: &str, rows: &[&SourceOccurrence<'_>], search: &str) -> bool {
    id.to_lowercase().contains(search)
        || rows.iter().any(|row| {
            row.trace_source.to_lowercase().contains(search)
                || row
                    .source_version
                    .is_some_and(|version| version.to_lowercase().contains(search))
        })
}

fn compare_profiles(
    request: &Request,
    left: Profiles<'_, '_>,
    right: Profiles<'_, '_>,
    measurements: &BTreeMap<Input<'_>, Measurements<'_>>,
) -> Value {
    let mut aligned = BTreeMap::<_, [Option<Counts<'_>>; 2]>::new();
    for (side, cohort) in [left, right].into_iter().enumerate() {
        for (id, rows) in cohort {
            aligned.entry(id).or_default()[side] = Some(summarize(id, &rows, measurements));
        }
    }
    let mut counts: Vec<_> = aligned.into_iter().collect();
    counts.sort_by_key(|(id, sides)| {
        (
            Reverse(
                sides
                    .iter()
                    .flatten()
                    .map(|row| row.call_count)
                    .max()
                    .unwrap_or_default(),
            ),
            *id,
        )
    });
    let rows: Vec<_> = counts
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|(id, [a, b])| json!({"source_id": id, "scope_a": a, "scope_b": b}))
        .collect();
    json!({
        "version": 1, "kind": "context_source_profile_comparison", "total_sources": counts.len(),
        "rows": rows, "next_offset": next_offset(request.offset, rows.len(), counts.len()),
    })
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
    source_id: &'a str,
    occurrences: &[&SourceOccurrence<'a>],
    measurements: &BTreeMap<Input<'_>, Measurements<'_>>,
) -> Counts<'a> {
    let mut calls = BTreeSet::new();
    let mut threads = BTreeSet::new();
    let mut users = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut decisions = BTreeSet::new();
    let mut versions = BTreeMap::<_, (usize, usize, BTreeSet<&str>)>::new();
    let (mut unversioned, mut included, mut replays, mut masks) = (0, 0, 0, 0);
    for row in occurrences {
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
        unversioned += usize::from(row.source_version.is_none());
        let (available, unavailable, forms) = versions.entry(row.source_version).or_default();
        if row.availability == Availability::Included {
            included += 1;
            *available += 1;
        } else {
            *unavailable += 1;
        }
        if let Some(form) = row.text_sha256.as_deref() {
            forms.insert(form);
        }
        if let Some(entry) = measurements.get(&input_key(row)) {
            replays += usize::from(entry.replay_count > 0);
            masks += usize::from(entry.mask_count > 0);
        }
    }
    Counts {
        source_id,
        occurrence_count: occurrences.len(),
        call_count: calls.len(),
        conversation_count: threads.len(),
        user_count: users.len(),
        organization_count: organizations.len(),
        linked_decision_count: decisions.len(),
        version_count: versions.keys().flatten().count(),
        unversioned_occurrences: unversioned,
        included_occurrences: included,
        replayed_occurrences: replays,
        masked_occurrences: masks,
        versions: versions
            .into_iter()
            .map(
                |(source_version, (included_occurrences, unavailable_occurrences, forms))| {
                    VersionCounts {
                        source_version,
                        included_occurrences,
                        unavailable_occurrences,
                        captured_forms: forms.len(),
                    }
                },
            )
            .collect(),
    }
}

fn occurrences(
    id: &str,
    rows: &[&SourceOccurrence<'_>],
    offset: usize,
    limit: usize,
    measurements: &BTreeMap<Input<'_>, Measurements<'_>>,
) -> Value {
    let totals = (!rows.is_empty()).then(|| summarize(id, rows, measurements));
    let mut items = rows.to_vec();
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
        .skip(offset)
        .take(limit)
        .map(|row| detail(row, measurements.get(&input_key(row))))
        .collect();
    json!({
        "version": 1, "kind": "context_source_occurrences", "source": totals,
        "total_occurrences": items.len(), "rows": rows,
        "next_offset": next_offset(offset, rows.len(), items.len()),
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
