//! Compare source visibility measurements across traces.

mod timeline;

use super::{query, BoundaryWhitespace, Report, Source};
use crate::{
    activation::{self, patterns::query::Thread},
    conflict_query::{timestamp, Scope, TimeWindow},
    Call, Run,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    output_token: Option<usize>,
    target: Option<String>,
    #[serde(default)]
    kind: Mask,
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    source_id: Option<String>,
    tool_name: Option<String>,
    after: Option<String>,
    before: Option<String>,
    compare: Option<Selection>,
    min_conversations: Option<usize>,
    #[serde(default)]
    order: GroupOrder,
    group: Option<String>,
    timeline: Option<timeline::Selection>,
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
    tool_name: Option<String>,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum GroupOrder {
    #[default]
    Recurrence,
    Magnitude,
}

impl GroupOrder {
    fn compare(
        self,
        (left_count, left_effect): (usize, f64),
        (right_count, right_effect): (usize, f64),
    ) -> Ordering {
        let recurrence = right_count.cmp(&left_count);
        let magnitude = right_effect.total_cmp(&left_effect);
        match self {
            Self::Recurrence => recurrence.then(magnitude),
            Self::Magnitude => magnitude.then(recurrence),
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Identity<'a> {
    source_id: &'a str,
    source_version: Option<&'a str>,
    kind: &'a str,
    role: Option<&'a str>,
    captured_text_sha256: &'a str,
    masked_token_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    boundary_whitespace: Option<&'a BoundaryWhitespace>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Comparison<'a> {
    model_id: &'a str,
    model_revision: &'a str,
    runtime: &'a str,
    tokenizer_sha256: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_token: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_prefix_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_kind: Option<&'a str>,
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
    output: [usize; 2],
    target_prefix: Option<String>,
}

type ThreadKey<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str);

#[derive(Default)]
struct Group<'a> {
    occurrences: Vec<Occurrence<'a>>,
    conversations: BTreeSet<ThreadKey<'a>>,
    maximum_absolute_effect: f64,
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
        self.maximum_absolute_effect = self.maximum_absolute_effect.max(occurrence.effect.abs());
        self.occurrences.push(occurrence);
    }

    fn rank(&self) -> (usize, f64) {
        (self.conversations.len(), self.maximum_absolute_effect)
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
    if request.output_token.is_some() == request.target.is_some()
        || request
            .target
            .as_deref()
            .is_some_and(|key| key.is_empty() || key.trim() != key)
    {
        return Err("Select one output token or one named output target.".into());
    }
    if request.timeline.is_some() && request.group.is_none() {
        return Err("Select a visibility pattern group to query its timeline.".into());
    }
    let primary_tool = request.tool_name.as_deref();
    let secondary_tool = request
        .compare
        .as_ref()
        .and_then(|side| side.tool_name.as_deref());
    if [primary_tool, secondary_tool]
        .into_iter()
        .flatten()
        .any(|name| name.is_empty() || name.trim() != name)
    {
        return Err("Use a nonempty recorded tool name without surrounding whitespace.".into());
    }
    let runs: Vec<_> = runs.into_iter().collect();
    let window = TimeWindow::parse(request.after.as_deref(), request.before.as_deref())?;
    let scoped = scope_runs(&runs, &request.scope, request.thread.as_ref());
    let primary = gather(request, scoped, reports, &window, primary_tool)?;
    let metric = match request.kind {
        Mask::Source => "change_in_log_probability",
        Mask::Pair => "interaction_log_units",
    };
    if let Some(compare) = &request.compare {
        let window = TimeWindow::parse(compare.after.as_deref(), compare.before.as_deref())?;
        let scoped = scope_runs(&runs, &compare.scope, compare.thread.as_ref());
        let secondary = gather(request, scoped, reports, &window, secondary_tool)?;
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
    tool_name: Option<&str>,
) -> Result<Cohort<'a>, String> {
    let exact = request.target.is_none();
    let mut groups = BTreeMap::<Comparison<'_>, Group<'_>>::new();
    let mut reports_in_scope = 0;
    let mut reports_matching = 0;
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
        if !window.includes(time) {
            continue;
        }
        let Some(decision) = call
            .decisions
            .iter()
            .find(|decision| decision.id == report.location.decision_id)
        else {
            continue;
        };
        if tool_name.is_some_and(|name| decision.kind != "tool_call" || decision.label != name) {
            continue;
        }
        reports_in_scope += 1;
        let target = request
            .target
            .as_deref()
            .and_then(|key| report.targets.iter().find(|target| target.key == key));
        let token_range = if let Some(index) = request.output_token {
            (index < report.output_token_ids.len()).then(|| [index, index + 1])
        } else {
            target.map(|target| target.token_range)
        };
        let Some([start, end]) = token_range else {
            continue;
        };
        reports_matching += 1;
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
                        boundary_whitespace: source.boundary_whitespace.as_ref(),
                    },
                ))
            })
            .collect();
        let mut prefix = Sha256::new();
        for id in &report.output_token_ids[..end] {
            prefix.update(id.to_be_bytes());
        }
        let prefix = format!("{:x}", prefix.finalize());
        let comparison = Comparison {
            model_id: &report.model_id,
            model_revision: &report.model_revision,
            runtime: &report.runtime,
            tokenizer_sha256: &report.tokenizer_sha256,
            decision_label: exact.then_some(decision.label.as_str()),
            output_token: exact.then_some(start),
            output_prefix_sha256: exact.then(|| prefix.clone()),
            target: target.map(|target| target.key.as_str()),
            decision_kind: report
                .baseline_tool_call
                .as_ref()
                .map(|_| decision.kind.as_str()),
            sources: Vec::new(),
        };
        let mut record = |context_ids: Vec<&'a str>, pair, effect: f64| {
            let Some(mut sources) = context_ids
                .iter()
                .map(|id| identities.get(id).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                measurements_without_source_id += 1;
                return Ok(());
            };
            if request
                .source_id
                .as_deref()
                .is_some_and(|id| !sources.iter().any(|source| source.source_id == id))
            {
                return Ok(());
            }
            if !effect.is_finite() {
                return Err("An output target has a nonfinite effect.".to_string());
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
                output: [start, end],
                target_prefix: (!exact).then(|| prefix.clone()),
            });
            Ok(())
        };
        match request.kind {
            Mask::Source => {
                for source in measured.values() {
                    record(
                        vec![&source.context_id],
                        None,
                        (start..end)
                            .map(|index| {
                                source.masked_log_probs[index] - report.baseline_log_probs[index]
                            })
                            .sum(),
                    )?;
                }
            }
            Mask::Pair => {
                for (id, pair) in report.pairs.iter().enumerate() {
                    record(
                        pair.context_ids.iter().map(String::as_str).collect(),
                        Some(id),
                        (start..end)
                            .map(|index| {
                                query::pair_scores(
                                    pair,
                                    &measured,
                                    report.baseline_log_probs[index],
                                    index,
                                )
                                .interaction
                            })
                            .sum(),
                    )?;
                }
            }
        }
    }
    let mut coverage = json!({"measurements_without_source_id": measurements_without_source_id});
    if exact {
        coverage["reports_at_token"] = json!(reports_matching);
    } else {
        coverage["reports_with_target"] = json!(reports_matching);
        coverage["reports_in_scope"] = json!(reports_in_scope);
    }
    Ok(Cohort { groups, coverage })
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
            return group_page(
                request,
                key,
                &comparison,
                [Some(group), None],
                [coverage, Value::Null],
                metric,
            );
        }
        return Err("The visibility pattern is unavailable in this scope.".into());
    }
    groups.sort_by(|(_, a), (_, b)| request.order.compare(a.rank(), b.rank()));
    let total = groups.len();
    let rows: Vec<_> = groups
        .iter()
        .skip(request.offset)
        .take(request.limit)
        .map(|(comparison, group)| Ok(summary(comparison, group, &group_key(comparison)?)))
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "kind": "visibility_pattern_groups", "metric": metric, "coverage": coverage, "order": request.order,
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
    let rank = |sides: &[Option<Group<'_>>; 2]| {
        sides.iter().flatten().map(Group::rank).fold(
            (0, 0.0_f64),
            |(count, effect), (side_count, side_effect)| {
                (count.max(side_count), effect.max(side_effect))
            },
        )
    };
    let mut groups: Vec<_> = combined
        .into_iter()
        .filter(|(_, sides)| rank(sides).0 >= request.min_conversations.unwrap_or_default())
        .collect();
    if let Some(key) = &request.group {
        for (comparison, sides) in groups {
            if group_key(&comparison)? != *key {
                continue;
            }
            return group_page(
                request,
                key,
                &comparison,
                sides,
                [a.coverage, b.coverage],
                metric,
            );
        }
        return Err("The visibility pattern is unavailable in either comparison scope.".into());
    }
    groups.sort_by(|(_, a), (_, b)| request.order.compare(rank(a), rank(b)));
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
        "kind": "visibility_pattern_comparison", "metric": metric, "order": request.order,
        "coverage_a": a.coverage, "coverage_b": b.coverage,
        "total_groups": total, "groups": rows, "next_offset": next_offset(request.offset, rows.len(), total),
    }))
}

fn group_page(
    request: &Request,
    key: &str,
    comparison: &Comparison<'_>,
    mut sides: [Option<Group<'_>>; 2],
    [coverage_a, coverage_b]: [Value; 2],
    metric: &str,
) -> Result<Value, String> {
    if let Some(timeline) = &request.timeline {
        if let Some(period) = timeline.period.as_deref() {
            sides = sides.map(|group| group.and_then(|group| timeline.filter(group, period)));
            if sides.iter().all(Option::is_none) {
                return Err("The visibility pattern has no measurements in this period.".into());
            }
        } else {
            let mut page = timeline.overview(sides, request.offset, request.limit);
            page["group"] = json!({"key": key, "comparison": comparison});
            page["metric"] = json!(metric);
            page["coverage_a"] = coverage_a;
            page["coverage_b"] = coverage_b;
            return Ok(page);
        }
    }
    let [left, right] = sides;
    if request.compare.is_some() {
        return Ok(json!({
            "kind": "visibility_pattern_comparison_evidence", "metric": metric,
            "group": {"key": key, "comparison": comparison},
            "scope_a": left.map(|group| occurrence_page(comparison, group, key, request.offset, request.limit)),
            "scope_b": right.map(|group| occurrence_page(comparison, group, key, request.compare_offset, request.limit)),
            "coverage_a": coverage_a, "coverage_b": coverage_b,
        }));
    }
    let group = left.ok_or("The visibility pattern is unavailable in this scope.")?;
    let mut page = occurrence_page(comparison, group, key, request.offset, request.limit);
    page["kind"] = json!("visibility_pattern_evidence");
    page["metric"] = json!(metric);
    page["coverage"] = coverage_a;
    Ok(page)
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
        .map(evidence)
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
    let mut result = group_counts(group);
    result["key"] = json!(key);
    result["comparison"] = json!(comparison);
    result
}

fn group_counts(group: &Group<'_>) -> Value {
    let mut decisions = BTreeMap::new();
    let mut users = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut prefixes = BTreeSet::new();
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
        if let Some(prefix) = item.target_prefix.as_deref() {
            prefixes.insert(prefix);
        }
        let signs = decisions
            .entry(&item.report.location)
            .or_insert((true, true));
        signs.0 &= item.effect < 0.0;
        signs.1 &= item.effect > 0.0;
    }
    let mut result = json!({
        "decision_count": decisions.len(),
        "conversation_count": group.conversations.len(), "measurement_count": group.occurrences.len(),
        "user_count": users.len(), "organization_count": organizations.len(),
        "negative_decisions": decisions.values().filter(|signs| signs.0).count(),
        "positive_decisions": decisions.values().filter(|signs| signs.1).count(),
        "effect": {"minimum": minimum, "maximum": maximum},
    });
    if !prefixes.is_empty() {
        result["output_prefix_count"] = json!(prefixes.len());
    }
    result
}

fn evidence(item: &Occurrence<'_>) -> Value {
    let [start, end] = item.output;
    let inspector_path = match item.pair {
        Some(pair) => query::visibility_path(item.report, start, Some(pair)),
        None => query::source_mask_path(item.report, start, item.context_ids[0]),
    };
    let mut row = json!({
        "location": item.report.location, "report_id": item.report.id, "context_ids": item.context_ids,
        "trace_source": item.run.source, "organization_id": item.run.organization_id,
        "user_id": item.run.user_id, "conversation_id": item.run.conversation_id,
        "turn_id": item.call.turn_id, "started_at": item.call.started_at, "output_token": start,
        "effect": item.effect, "inspector_path": inspector_path,
        "source_inspector_paths": item.context_ids.iter().map(|id| query::source_path(item.report, id)).collect::<Vec<_>>(),
    });
    if let Some(prefix) = &item.target_prefix {
        row["output_token_end"] = json!(end);
        row["output_prefix_sha256"] = json!(prefix);
    }
    if let Some(proposal) = &item.report.baseline_tool_call {
        row["tool_name"] = json!(proposal.name);
    }
    row
}
