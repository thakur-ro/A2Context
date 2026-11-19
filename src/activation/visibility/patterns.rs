//! Compare source visibility measurements at an exact output token across traces.

use super::{query, Report, Source};
use crate::{
    activation::{self, patterns::query::Thread},
    conflict_query::{timestamp, Scope, TimeWindow},
    Call, Run,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    output_token: usize,
    #[serde(default)]
    kind: Mask,
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    source_id: Option<String>,
    after: Option<String>,
    before: Option<String>,
    compare: Option<Selection>,
    min_conversations: Option<usize>,
    group: Option<String>,
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mask {
    #[default]
    Source,
    Pair,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Identity<'a> {
    source_id: &'a str,
    source_version: Option<&'a str>,
    kind: &'a str,
    role: Option<&'a str>,
    captured_text_sha256: &'a str,
    masked_token_count: usize,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Comparison<'a> {
    model_id: &'a str,
    model_revision: &'a str,
    runtime: &'a str,
    tokenizer_sha256: &'a str,
    decision_label: &'a str,
    output_token: usize,
    output_prefix_sha256: String,
    sources: Vec<Identity<'a>>,
}

struct Occurrence<'a> {
    run: &'a Run,
    call: &'a Call,
    report: &'a Report,
    time: Option<OffsetDateTime>,
    context_ids: Vec<&'a str>,
    pair: Option<usize>,
    effect: f64,
}

type ThreadKey<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str);

#[derive(Default)]
struct Group<'a> {
    occurrences: Vec<Occurrence<'a>>,
    conversations: BTreeSet<ThreadKey<'a>>,
}

type Groups<'a> = BTreeMap<Comparison<'a>, Group<'a>>;

struct Cohort<'a> {
    groups: Groups<'a>,
    coverage: Value,
}

impl<'a> Group<'a> {
    fn add(&mut self, occurrence: Occurrence<'a>) {
        let run = occurrence.run;
        if let Some(thread) = run.conversation_id.as_deref() {
            self.conversations.insert((
                &run.source,
                run.organization_id.as_deref(),
                run.user_id.as_deref(),
                thread,
            ));
        }
        self.occurrences.push(occurrence);
    }
}

/// Compare reports already validated against the runs available to the caller.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    reports: &'a [activation::Report],
) -> Result<Value, String> {
    if request.version != 1 || !(1..=50).contains(&request.limit) {
        return Err("Use visibility pattern query version 1 and a page size from 1 to 50.".into());
    }
    let runs: Vec<_> = runs.into_iter().collect();
    let window = TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?;
    let scoped = scope_runs(&runs, &request.scope, request.thread.as_ref());
    let primary = gather(request, scoped, reports, &window);
    let metric = match request.kind {
        Mask::Source => "change_in_log_probability",
        Mask::Pair => "interaction_log_units",
    };
    if let Some(compare) = &request.compare {
        let window = TimeWindow::parse(compare.after.as_deref(), compare.before.as_deref())?;
        let scoped = scope_runs(&runs, &compare.scope, compare.thread.as_ref());
        let secondary = gather(request, scoped, reports, &window);
        compare_cohorts(request, primary, secondary, metric)
    } else {
        single_cohort(request, primary, metric)
    }
}

fn scope_runs<'a>(
    runs: &[&'a Run],
    scope: &Scope,
    thread: Option<&Thread>,
) -> BTreeMap<&'a str, &'a Run> {
    runs.iter()
        .copied()
        .filter(|run| scope.includes(run) && thread.is_none_or(|thread| thread.includes(run)))
        .map(|run| (run.id.as_str(), run))
        .collect()
}

fn gather<'a>(
    request: &Request,
    scoped: BTreeMap<&'a str, &'a Run>,
    reports: &'a [activation::Report],
    window: &TimeWindow,
) -> Cohort<'a> {
    let index = request.output_token;
    let mut groups = BTreeMap::<Comparison<'_>, Group<'_>>::new();
    let mut reports_at_token = 0;
    let mut measurements_without_source_id = 0;
    for entry in reports {
        let activation::Report::Visibility(report) = entry else {
            continue;
        };
        let Some(run) = scoped.get(report.location.run_id.as_str()).copied() else {
            continue;
        };
        let Some(call) = run
            .calls
            .iter()
            .find(|call| call.id == report.location.call_id)
        else {
            continue;
        };
        let time = call
            .started_at
            .as_deref()
            .and_then(|value| timestamp(value).ok());
        if !window.includes(time) || index >= report.output_token_ids.len() {
            continue;
        }
        let Some(decision) = call
            .decisions
            .iter()
            .find(|decision| decision.id == report.location.decision_id)
        else {
            continue;
        };
        reports_at_token += 1;
        let measured: BTreeMap<_, _> = report
            .sources
            .iter()
            .filter_map(Source::measured)
            .map(|source| (source.context_id.as_str(), source))
            .collect();
        let identities: BTreeMap<_, _> = call
            .context
            .iter()
            .filter_map(|item| {
                let source = measured.get(item.id.as_str())?;
                let id = item
                    .source_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())?;
                Some((
                    item.id.as_str(),
                    Identity {
                        source_id: id,
                        source_version: item.source_version.as_deref(),
                        kind: &item.kind,
                        role: item.role.as_deref(),
                        captured_text_sha256: &source.captured_text_sha256,
                        masked_token_count: source.token_range[1] - source.token_range[0],
                    },
                ))
            })
            .collect();
        let mut prefix = Sha256::new();
        for id in &report.output_token_ids[..=index] {
            prefix.update(id.to_be_bytes());
        }
        let comparison = Comparison {
            model_id: &report.model_id,
            model_revision: &report.model_revision,
            runtime: &report.runtime,
            tokenizer_sha256: &report.tokenizer_sha256,
            decision_label: &decision.label,
            output_token: index,
            output_prefix_sha256: format!("{:x}", prefix.finalize()),
            sources: Vec::new(),
        };
        let mut record = |context_ids: Vec<&'a str>, pair, effect| {
            let Some(mut sources) = context_ids
                .iter()
                .map(|id| identities.get(id).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                measurements_without_source_id += 1;
                return;
            };
            if request
                .source_id
                .as_deref()
                .is_some_and(|id| !sources.iter().any(|source| source.source_id == id))
            {
                return;
            }
            sources.sort_unstable();
            let key = Comparison {
                sources,
                ..comparison.clone()
            };
            groups.entry(key).or_default().add(Occurrence {
                run,
                call,
                report,
                time,
                context_ids,
                pair,
                effect,
            });
        };
        let baseline = report.baseline_log_probs[index];
        match request.kind {
            Mask::Source => {
                for source in measured.values() {
                    record(
                        vec![&source.context_id],
                        None,
                        source.masked_log_probs[index] - baseline,
                    );
                }
            }
            Mask::Pair => {
                for (id, pair) in report.pairs.iter().enumerate() {
                    let scores = query::pair_scores(pair, &measured, baseline, index);
                    record(
                        pair.context_ids.iter().map(String::as_str).collect(),
                        Some(id),
                        scores.interaction,
                    );
                }
            }
        }
    }
    Cohort {
        groups,
        coverage: json!({"reports_at_token": reports_at_token, "measurements_without_source_id": measurements_without_source_id}),
    }
}

fn single_cohort(request: &Request, cohort: Cohort<'_>, metric: &str) -> Result<Value, String> {
    let Cohort { groups, coverage } = cohort;
    let mut groups: Vec<_> = groups
        .into_iter()
        .filter(|(_, group)| {
            group.conversations.len() >= request.min_conversations.unwrap_or_default()
        })
        .collect();
    if let Some(key) = &request.group {
        for (comparison, group) in groups {
            if group_key(&comparison)? != *key {
                continue;
            }
            let mut page = occurrence_page(&comparison, group, key, request.offset, request.limit);
            page["kind"] = json!("visibility_pattern_evidence");
            page["metric"] = json!(metric);
            page["coverage"] = coverage;
            return Ok(page);
        }
        return Err("The visibility pattern is unavailable in this scope.".into());
    }
    groups.sort_by_key(|(_, group)| Reverse(group.conversations.len()));
    let total = groups.len();
    let rows: Vec<_> = groups
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|(comparison, group)| Ok(summary(comparison, group, &group_key(comparison)?)))
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "kind": "visibility_pattern_groups", "metric": metric, "coverage": coverage,
        "total_groups": total, "groups": rows, "next_offset": next_offset(request.offset, rows.len(), total),
    }))
}

fn compare_cohorts(
    request: &Request,
    a: Cohort<'_>,
    b: Cohort<'_>,
    metric: &str,
) -> Result<Value, String> {
    let mut combined = BTreeMap::<_, [Option<Group<'_>>; 2]>::new();
    for (index, groups) in [a.groups, b.groups].into_iter().enumerate() {
        for (key, group) in groups {
            combined.entry(key).or_default()[index] = Some(group);
        }
    }
    let size = |sides: &[Option<Group<'_>>; 2]| {
        sides
            .iter()
            .flatten()
            .map(|group| group.conversations.len())
            .max()
            .unwrap_or_default()
    };
    let mut groups: Vec<_> = combined
        .into_iter()
        .filter(|(_, sides)| size(sides) >= request.min_conversations.unwrap_or_default())
        .collect();
    if let Some(key) = &request.group {
        for (comparison, [left, right]) in groups {
            if group_key(&comparison)? != *key {
                continue;
            }
            return Ok(json!({
                "kind": "visibility_pattern_comparison_evidence", "metric": metric,
                "group": {"key": key, "comparison": comparison},
                "scope_a": left.map(|group| occurrence_page(&comparison, group, key, request.offset, request.limit)),
                "scope_b": right.map(|group| occurrence_page(&comparison, group, key, request.compare_offset, request.limit)),
                "coverage_a": a.coverage, "coverage_b": b.coverage,
            }));
        }
        return Err("The visibility pattern is unavailable in either comparison scope.".into());
    }
    groups.sort_by_key(|(_, sides)| Reverse(size(sides)));
    let total = groups.len();
    let rows: Vec<_> = groups
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|(comparison, sides)| {
            let key = group_key(comparison)?;
            Ok(json!({
                "key": key, "comparison": comparison,
                "scope_a": sides[0].as_ref().map(|group| summary(comparison, group, &key)),
                "scope_b": sides[1].as_ref().map(|group| summary(comparison, group, &key)),
            }))
        })
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "kind": "visibility_pattern_comparison", "metric": metric,
        "coverage_a": a.coverage, "coverage_b": b.coverage,
        "total_groups": total, "groups": rows, "next_offset": next_offset(request.offset, rows.len(), total),
    }))
}

fn occurrence_page(
    comparison: &Comparison<'_>,
    mut group: Group<'_>,
    key: &str,
    offset: usize,
    limit: usize,
) -> Value {
    group.occurrences.sort_by_key(|item| {
        (
            item.time.is_none(),
            item.time,
            &item.report.location,
            &item.report.id,
        )
    });
    let total = group.occurrences.len();
    let rows: Vec<_> = group
        .occurrences
        .iter()
        .skip(offset)
        .take(limit)
        .map(|item| evidence(item, comparison.output_token))
        .collect();
    json!({
        "group": summary(comparison, &group, key), "total_occurrences": total,
        "occurrences": rows, "next_offset": next_offset(offset, rows.len(), total),
    })
}

fn group_key(comparison: &Comparison<'_>) -> Result<String, String> {
    let data = serde_json::to_vec(comparison).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(data)))
}

fn next_offset(offset: usize, length: usize, total: usize) -> Option<usize> {
    let next = offset.saturating_add(length);
    (next < total).then_some(next)
}

fn summary(comparison: &Comparison<'_>, group: &Group<'_>, key: &str) -> Value {
    let mut decisions = BTreeMap::new();
    let mut users = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for item in &group.occurrences {
        let organization = item.run.organization_id.as_deref();
        if let Some(organization) = organization {
            organizations.insert(organization);
        }
        if let Some(user) = item.run.user_id.as_deref() {
            users.insert((organization, user));
        }
        minimum = minimum.min(item.effect);
        maximum = maximum.max(item.effect);
        let signs = decisions
            .entry(&item.report.location)
            .or_insert((true, true));
        signs.0 &= item.effect < 0.0;
        signs.1 &= item.effect > 0.0;
    }
    json!({
        "key": key, "comparison": comparison, "decision_count": decisions.len(),
        "conversation_count": group.conversations.len(), "measurement_count": group.occurrences.len(),
        "user_count": users.len(), "organization_count": organizations.len(),
        "negative_decisions": decisions.values().filter(|signs| signs.0).count(),
        "positive_decisions": decisions.values().filter(|signs| signs.1).count(),
        "effect": {"minimum": minimum, "maximum": maximum},
    })
}

fn evidence(item: &Occurrence<'_>, index: usize) -> Value {
    json!({
        "location": item.report.location, "report_id": item.report.id, "context_ids": item.context_ids,
        "trace_source": item.run.source, "organization_id": item.run.organization_id,
        "user_id": item.run.user_id, "conversation_id": item.run.conversation_id,
        "turn_id": item.call.turn_id, "started_at": item.call.started_at, "output_token": index,
        "effect": item.effect, "inspector_path": query::visibility_path(item.report, index, item.pair),
        "source_inspector_paths": item.context_ids.iter().map(|id| query::source_path(item.report, id)).collect::<Vec<_>>(),
    })
}
