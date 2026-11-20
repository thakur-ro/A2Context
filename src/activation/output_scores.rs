//! Check and read log probabilities along one captured output path.

use super::{Report, TimelineReport};
use crate::{replay::Location, turn_evidence};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct OutputScores {
    readout: Readout,
    fragments: Vec<String>,
    baseline_log_probs: Vec<f64>,
    sources: BTreeMap<String, SourceScores>,
    checks: Checks,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Readout {
    OutputTokenLogProbability,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SourceScores {
    edited_log_probs: Vec<f64>,
    block_log_probs: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Checks {
    layer: usize,
    source_context_id: String,
    clean_patch_log_probs: Vec<f64>,
    full_donor_log_probs: Vec<f64>,
}

impl OutputScores {
    pub(super) fn validate<'a>(
        &self,
        report: &TimelineReport,
        context_ids: impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let count = report.output_token_ids.len();
        let valid = |values: &[f64]| {
            values.len() == count
                && values
                    .iter()
                    .all(|value| value.is_finite() && *value <= 0.0)
        };
        if self.fragments.len() != count
            || self.fragments.concat() != report.baseline_answer
            || !valid(&self.baseline_log_probs)
        {
            return Err(
                "output scores must align with every captured output token and its text".into(),
            );
        }
        if !context_ids.eq(self.sources.keys().map(String::as_str))
            || self.sources.values().any(|source| {
                !valid(&source.edited_log_probs)
                    || source.block_log_probs.len() != report.layer_count
                    || source.block_log_probs.iter().any(|row| !valid(row))
            })
        {
            return Err(
                "output scores need one log probability per token, source edit, and decoder block"
                    .into(),
            );
        }
        let donor = self
            .sources
            .get(&self.checks.source_context_id)
            .ok_or("the output score control references an unknown source edit")?;
        let close = |measured: &[f64], expected: &[f64]| {
            valid(measured)
                && measured
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() <= 0.05)
        };
        if self.checks.layer >= report.layer_count
            || !close(&self.checks.clean_patch_log_probs, &self.baseline_log_probs)
            || !close(&self.checks.full_donor_log_probs, &donor.edited_log_probs)
        {
            return Err("the output scores failed their instrumentation controls".into());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    pub location: Location,
    report_id: String,
    block: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    include_excerpts: bool,
    #[serde(default)]
    include_edits: bool,
}

fn default_limit() -> usize {
    12
}

pub fn query(request: &Request, reports: &[Report], allow_excerpts: bool) -> Result<Value, String> {
    if request.version != 1 || !(1..=24).contains(&request.limit) {
        return Err("Use output score query version 1 and a page size from 1 to 24.".into());
    }
    if request.include_excerpts && !allow_excerpts {
        return Err("This MCP process has not enabled excerpts.".into());
    }
    if request.include_edits && !request.include_excerpts {
        return Err("Source edits require an explicit excerpt request.".into());
    }
    let (report, scores) = reports
        .iter()
        .find_map(|entry| match entry {
            Report::Timeline(report)
                if report.id == request.report_id && report.location == request.location =>
            {
                report.output_scores.as_ref().map(|scores| (report, scores))
            }
            _ => None,
        })
        .ok_or("The requested output scores are unavailable.")?;
    if request.block >= report.layer_count {
        return Err("The requested decoder block is unavailable for this report.".into());
    }
    let total = report.output_token_ids.len();
    let block = request.block;
    let rows: Vec<_> = (request.offset..total).take(request.limit).map(|index| {
        let baseline = scores.baseline_log_probs[index];
        // A page holds at most 24 positions. Look outside it only when comparing edits.
        let repeated = if request.include_edits {
            let ids = &report.output_token_ids;
            (index + 1..total).find(|&other| ids[other] == ids[index])
                .or_else(|| (0..index).rev().find(|&other| ids[other] == ids[index]))
        } else { None };
        let sources: Vec<_> = scores.sources.iter().map(|(id, source)| {
            let patched = source.block_log_probs[block][index];
            let mut result = json!({
                "context_id": id, "patched_log_probability": patched,
                "edited_prompt_log_probability": source.edited_log_probs[index],
                "change_in_log_probability": patched - baseline,
            });
            if let Some(other) = repeated {
                result["other_edited_delta"] = json!(source.edited_log_probs[other] - scores.baseline_log_probs[other]);
            }
            result
        }).collect();
        let mut row = json!({
            "output_token": index, "baseline_log_probability": baseline, "sources": sources,
            "inspector_path": super::activation_path(&report.location, &report.id, "token", index, block),
        });
        if request.include_excerpts {
            row["fragment"] = turn_evidence::excerpt(&scores.fragments[index]);
            row["token_id"] = json!(report.output_token_ids[index]);
        }
        if request.include_edits {
            row["other_occurrence"] = json!(repeated);
        }
        row
    }).collect();
    let next = request.offset.saturating_add(rows.len());
    let mut page = json!({
        "version": 1, "location": report.location, "report_id": report.id,
        "readout": scores.readout, "block": block, "total_output_tokens": total,
        "captured_excerpts_enabled": allow_excerpts,
        "tokens": rows, "next_offset": (next < total).then_some(next),
    });
    if request.include_edits {
        page["baseline_output"] = turn_evidence::excerpt(&report.baseline_answer);
        let edits: Vec<_> = report
            .sources
            .iter()
            .map(|source| {
                let edited = &scores.sources[&source.context_id].edited_log_probs;
                let peak = edited
                    .iter()
                    .zip(&scores.baseline_log_probs)
                    .enumerate()
                    .map(|(token, (edited, baseline))| (token, edited - baseline))
                    .filter(|(_, delta)| *delta != 0.0)
                    .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()).then_with(|| b.0.cmp(&a.0)));
                let tool_selection_changed = report.tool_calls.as_ref().map(|calls| {
                    calls.edited[&source.context_id]
                        .as_ref()
                        .map(|call| &call.name)
                        != Some(&calls.baseline.name)
                });
                json!({
                    "context_id": source.context_id,
                    "peak": peak.map(|(token, delta)| json!({"token": token, "delta": delta})),
                    "before": turn_evidence::excerpt(&source.before),
                    "after": turn_evidence::excerpt(&source.after),
                    "output": turn_evidence::excerpt(&source.edited_outcome),
                    "output_changed": source.edited_outcome != report.baseline_answer,
                    "generated_choice_changed": generated_choice_changed(report, source),
                    "tool_selection_changed": tool_selection_changed,
                })
            })
            .collect();
        page["edits"] = json!(edits);
    }
    Ok(page)
}

pub fn available(reports: &[Report]) -> bool {
    reports
        .iter()
        .any(|entry| matches!(entry, Report::Timeline(report) if report.output_scores.is_some()))
}

pub fn for_decision(
    reports: &[Report],
    run_id: &str,
    call_id: &str,
    decision_id: &str,
) -> Vec<Value> {
    reports.iter().filter_map(|entry| match entry {
        Report::Timeline(report) if report.output_scores.is_some()
            && report.location.run_id == run_id && report.location.call_id == call_id
            && report.location.decision_id == decision_id => Some(json!({
                "report_id": report.id, "total_output_tokens": report.output_token_ids.len(),
                "layer_count": report.layer_count, "model_id": report.model_id, "runtime": report.runtime,
            })),
        _ => None,
    }).collect()
}

// A tool proposal includes its arguments. Extra prose around an identical call can change independently.
fn generated_choice_changed(report: &TimelineReport, source: &super::Source<String>) -> bool {
    match &report.tool_calls {
        Some(calls) => calls.edited[&source.context_id].as_ref() != Some(&calls.baseline),
        None => source.edited_outcome != report.baseline_answer,
    }
}

/// The strongest measured source-edit effect on a token in the fixed output path.
#[derive(Serialize)]
pub struct Landmark<'a> {
    pub context_id: &'a str,
    pub model: &'a str,
    pub revision: &'a str,
    pub runtime: &'a str,
    pub token: usize,
    pub total_tokens: usize,
    pub generated_choice_changed: bool,
    pub block: usize,
    pub block_delta: f64,
    pub positions: Vec<LandmarkPosition<'a>>,
}

#[derive(Serialize)]
pub struct LandmarkPosition<'a> {
    pub token: usize,
    pub fragment: &'a str,
    pub baseline: f64,
    pub edited: f64,
}

/// Summarize an already validated instrumented report. The window follows a
/// fixed output path. It measures token scores under interventions.
pub fn landmark(entry: &Report) -> Option<Landmark<'_>> {
    let Report::Timeline(report) = entry else {
        return None;
    };
    let scores = report.output_scores.as_ref()?;
    let (context_id, source, token, _) = scores
        .sources
        .iter()
        .flat_map(|(id, source)| {
            source
                .edited_log_probs
                .iter()
                .zip(&scores.baseline_log_probs)
                .enumerate()
                .map(move |(token, (edited, baseline))| {
                    (id, source, token, (edited - baseline).abs())
                })
        })
        .filter(|(_, _, _, change)| *change > 0.0)
        .max_by(|left, right| left.3.total_cmp(&right.3))?;
    let baseline = *scores.baseline_log_probs.get(token)?;
    let (block, block_delta) = source
        .block_log_probs
        .iter()
        .enumerate()
        .filter_map(|(block, row)| Some((block, row.get(token)? - baseline)))
        .max_by(|left, right| left.1.abs().total_cmp(&right.1.abs()))?;
    let total = scores
        .fragments
        .len()
        .min(source.edited_log_probs.len())
        .min(scores.baseline_log_probs.len());
    if token >= total {
        return None;
    }
    let positions = (token.saturating_sub(2)..(token + 3).min(total))
        .map(|position| LandmarkPosition {
            token: position,
            fragment: &scores.fragments[position],
            baseline: scores.baseline_log_probs[position],
            edited: source.edited_log_probs[position],
        })
        .collect();
    Some(Landmark {
        context_id,
        model: &report.model_id,
        revision: &report.model_revision,
        runtime: &report.runtime,
        token,
        total_tokens: total,
        generated_choice_changed: generated_choice_changed(
            report,
            report
                .sources
                .iter()
                .find(|s| s.context_id == *context_id)?,
        ),
        block,
        block_delta,
        positions,
    })
}
