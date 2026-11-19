//! Read a measured source visibility report by token, source, or pair.

use super::{interaction, Measurement, Pair, Report, Source};
use crate::{activation, replay::Location, turn_evidence};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const UNAVAILABLE: &str = "The requested source visibility report is unavailable.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    pub location: Location,
    report_id: String,
    #[serde(default)]
    view: View,
    #[serde(default)]
    include_excerpts: bool,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum View {
    #[default]
    Timeline,
    Sources {
        output_token: usize,
        context_id: Option<String>,
    },
    Pairs {
        output_token: usize,
        context_ids: Option<[String; 2]>,
    },
}

fn default_limit() -> usize {
    12
}

pub fn available(reports: &[activation::Report]) -> bool {
    reports
        .iter()
        .any(|report| matches!(report, activation::Report::Visibility(_)))
}

pub(crate) struct SourceMask<'a> {
    report: &'a Report,
    source: &'a Measurement,
}

pub(crate) fn measured_masks(
    reports: &[activation::Report],
) -> impl Iterator<Item = SourceMask<'_>> {
    reports
        .iter()
        .filter_map(|entry| match entry {
            activation::Report::Visibility(report) => Some(report),
            _ => None,
        })
        .flat_map(|report| {
            report
                .sources
                .iter()
                .filter_map(Source::measured)
                .map(move |source| SourceMask { report, source })
        })
}

impl<'a> SourceMask<'a> {
    pub(crate) fn input(&self) -> (&'a Location, &'a str) {
        (&self.report.location, &self.source.context_id)
    }

    pub(crate) fn reference(&self) -> Value {
        let (token, effect) = self
            .source
            .masked_log_probs
            .iter()
            .enumerate()
            .rev()
            .map(|(token, masked)| (token, masked - self.report.baseline_log_probs[token]))
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .expect("validated source report has at least one token");
        json!({
            "location": self.report.location, "report_id": self.report.id,
            "output_token": token, "change_in_log_probability": effect,
            "inspector_path": source_mask_path(self.report, token, &self.source.context_id),
            "source_inspector_path": source_path(self.report, &self.source.context_id),
        })
    }
}

pub fn for_decision(
    reports: &[activation::Report],
    run: &str,
    call: &str,
    decision: &str,
) -> Vec<Value> {
    reports
        .iter()
        .filter_map(|entry| match entry {
            activation::Report::Visibility(report)
                if report.location.run_id == run
                    && report.location.call_id == call
                    && report.location.decision_id == decision =>
            {
                Some(summary(report))
            }
            _ => None,
        })
        .collect()
}

fn summary(report: &Report) -> Value {
    let mut result = json!({
        "report_id": report.id, "model_id": report.model_id, "model_revision": report.model_revision, "runtime": report.runtime,
        "total_output_tokens": report.output_token_ids.len(), "captured_sources": report.sources.len(),
        "measured_sources": report.sources.iter().filter_map(Source::measured).count(),
        "measured_pairs": report.pairs.len(),
        "all_visible_max_abs_error": report.all_visible_max_abs_error,
    });
    if !report.targets.is_empty() {
        result["targets"] = json!(report.targets);
    }
    if report.baseline_tool_call.is_some() {
        result["decision_kind"] = json!("tool_call");
    }
    result
}

/// Read reports that have been validated against their captured runs.
pub fn query(
    request: &Request,
    reports: &[activation::Report],
    allow_excerpts: bool,
) -> Result<Value, String> {
    if request.version != 1 || !(1..=24).contains(&request.limit) {
        return Err("Use source visibility query version 1 and a page size from 1 to 24.".into());
    }
    if request.include_excerpts && !allow_excerpts {
        return Err("This MCP process has not enabled excerpts.".into());
    }
    let report = reports
        .iter()
        .find_map(|entry| match entry {
            activation::Report::Visibility(report)
                if report.id == request.report_id && report.location == request.location =>
            {
                Some(report)
            }
            _ => None,
        })
        .ok_or(UNAVAILABLE)?;
    let measured: BTreeMap<_, _> = report
        .sources
        .iter()
        .filter_map(Source::measured)
        .map(|source| (source.context_id.as_str(), source))
        .collect();
    let token_count = report.output_token_ids.len();
    let (kind, total, token, rows) = match request.view {
        View::Timeline => {
            let rows = (request.offset..token_count)
                .take(request.limit)
                .map(|index| {
                    let baseline = report.baseline_log_probs[index];
                    let mut row = token_row(report, index, request.include_excerpts);
                    let strongest_source = report
                        .sources
                        .iter()
                        .rev()
                        .filter_map(Source::measured)
                        .map(|source| (source, source.masked_log_probs[index] - baseline))
                        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
                    let strongest_pair = report
                        .pairs
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(id, pair)| (id, pair, pair_scores(pair, &measured, baseline, index)))
                        .max_by(|a, b| a.2.interaction.abs().total_cmp(&b.2.interaction.abs()));
                    row["strongest_source"] =
                        json!(strongest_source.map(|(source, change)| json!({
                            "context_id": source.context_id, "change_in_log_probability": change,
                            "inspector_path": source_mask_path(report, index, &source.context_id),
                            "source_inspector_path": source_path(report, &source.context_id),
                        })));
                    row["strongest_pair"] = json!(strongest_pair.map(|(id, pair, scores)| json!({
                    "context_ids": pair.context_ids, "interaction_log_units": scores.interaction,
                    "inspector_path": visibility_path(report, index, Some(id)),
                })));
                    row
                })
                .collect();
            ("visibility_timeline", token_count, None, rows)
        }
        View::Sources {
            output_token,
            ref context_id,
        } if output_token < token_count => {
            let baseline = report.baseline_log_probs[output_token];
            let mut sources: Vec<_> = report
                .sources
                .iter()
                .filter(|source| context_id.as_deref().is_none_or(|id| source.id() == id))
                .map(|source| {
                    let magnitude = source.measured().map_or(-1.0, |source| {
                        (source.masked_log_probs[output_token] - baseline).abs()
                    });
                    (source, magnitude)
                })
                .collect();
            sources.sort_by(|a, b| b.1.total_cmp(&a.1));
            let rows = sources.iter().skip(request.offset).take(request.limit).map(|(source, _)| {
                let mut row = json!({"context_id": source.id(), "source_inspector_path": source_path(report, source.id())});
                match source {
                    Source::Measured(source) => {
                        row["token_range"] = json!(source.token_range);
                        row["effect"] = mask_effect(baseline, source.masked_log_probs[output_token]);
                        row["inspector_path"] = source_mask_path(report, output_token, &source.context_id).into();
                    }
                    Source::Skipped(source) => row["unavailable"] = json!(source.unavailable),
                }
                row
            }).collect();
            (
                "visibility_sources",
                sources.len(),
                Some(output_token),
                rows,
            )
        }
        View::Pairs {
            output_token,
            ref context_ids,
        } if output_token < token_count => {
            let baseline = report.baseline_log_probs[output_token];
            let mut ranked: Vec<_> = report
                .pairs
                .iter()
                .enumerate()
                .filter(|(_, pair)| {
                    context_ids.as_ref().is_none_or(|ids| {
                        *ids == pair.context_ids
                            || [&ids[1], &ids[0]] == pair.context_ids.each_ref()
                    })
                })
                .map(|(id, pair)| {
                    (
                        id,
                        pair,
                        pair_scores(pair, &measured, baseline, output_token),
                    )
                })
                .collect();
            ranked.sort_by(|a, b| b.2.interaction.abs().total_cmp(&a.2.interaction.abs()));
            let rows = ranked.iter().skip(request.offset).take(request.limit).map(|(id, pair, scores)| json!({
                "context_ids": pair.context_ids,
                "only_first": mask_effect(baseline, scores.first), "only_second": mask_effect(baseline, scores.second),
                "both_masked": mask_effect(baseline, scores.joint), "interaction_log_units": scores.interaction,
                "inspector_path": visibility_path(report, output_token, Some(*id)),
                "source_inspector_paths": pair.context_ids.each_ref().map(|context| source_path(report, context)),
            })).collect();
            ("visibility_pairs", ranked.len(), Some(output_token), rows)
        }
        _ => return Err("The requested output token is unavailable.".into()),
    };
    let rows: Vec<Value> = rows;
    let next = request.offset.saturating_add(rows.len());
    let mut result = json!({
        "version": 1, "kind": kind, "location": report.location, "report": summary(report),
        "readout": "fixed_prefix_token_log_probability", "captured_excerpts_enabled": allow_excerpts,
        "total_rows": total, "rows": rows,
        "next_offset": (next < total).then_some(next),
    });
    if let Some(index) = token {
        result["token"] = token_row(report, index, request.include_excerpts);
    }
    Ok(result)
}

pub(super) struct PairScores {
    first: f64,
    second: f64,
    joint: f64,
    pub(super) interaction: f64,
}

pub(super) fn pair_scores(
    pair: &Pair,
    measured: &BTreeMap<&str, &Measurement>,
    baseline: f64,
    index: usize,
) -> PairScores {
    let [first, second] = pair
        .context_ids
        .each_ref()
        .map(|source| measured[source.as_str()].masked_log_probs[index]);
    let joint = pair.masked_log_probs[index];
    PairScores {
        first,
        second,
        joint,
        interaction: interaction(baseline, first, second, joint),
    }
}

fn mask_effect(baseline: f64, masked: f64) -> Value {
    json!({"masked_log_probability": masked, "change_in_log_probability": masked - baseline})
}

fn token_row(report: &Report, index: usize, include_excerpts: bool) -> Value {
    let mut row = json!({
        "output_token": index, "baseline_log_probability": report.baseline_log_probs[index],
        "inspector_path": visibility_path(report, index, None),
    });
    if include_excerpts {
        row["fragment"] = turn_evidence::excerpt(&report.fragments[index]);
        row["token_id"] = json!(report.output_token_ids[index]);
    }
    row
}

pub(super) fn visibility_path(report: &Report, token: usize, pair: Option<usize>) -> String {
    let mut path = format!(
        "{}&visibility={token}",
        activation::activation_report_path(&report.location, &report.id)
    );
    if let Some(pair) = pair {
        path.push_str(&format!("&pair={pair}"));
    }
    path
}

pub(super) fn source_path(report: &Report, context: &str) -> String {
    let path = activation::decision_path(&report.location);
    let id = utf8_percent_encode(context, NON_ALPHANUMERIC);
    format!("{path}&context={id}")
}

pub(super) fn source_mask_path(report: &Report, token: usize, context: &str) -> String {
    let path = visibility_path(report, token, None);
    let id = utf8_percent_encode(context, NON_ALPHANUMERIC);
    format!("{path}&mask={id}")
}
