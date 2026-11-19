//! Inspect greedy continuations measured after a source-token activation swap.

use super::{activation_path, Report, TimelineReport};
use crate::{replay::Location, turn_evidence};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Branch {
    source_context_id: String,
    layer: usize,
    start_output_token: usize,
    max_new_tokens: usize,
    clean: Continuation,
    clean_patch_steps: usize,
    patched: Continuation,
    patched_steps: usize,
    edited_prompt: Continuation,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Continuation {
    pub(super) text: String,
    pub(super) token_ids: Vec<u32>,
    pub(super) stop_reason: StopReason,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StopReason {
    EndOfSequence,
    TokenLimit,
}

impl Continuation {
    pub(super) fn steps(&self, prefix: &[u32], maximum: usize) -> Option<usize> {
        let generated = self.token_ids.strip_prefix(prefix)?.len();
        match self.stop_reason {
            StopReason::EndOfSequence if generated < maximum => Some(generated + 1),
            StopReason::TokenLimit if generated == maximum => Some(generated),
            _ => None,
        }
    }
}

pub(super) fn validate(
    report: &TimelineReport,
    branches: &[Branch],
    sources: BTreeSet<&str>,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if branches.is_empty() {
        return Err("a generation branch report needs a measured continuation".into());
    }
    for branch in branches {
        let source_id = branch.source_context_id.as_str();
        let start = branch.start_output_token;
        if !sources.contains(source_id)
            || branch.layer >= report.layer_count
            || start >= report.output_token_ids.len()
            || branch.max_new_tokens == 0
            || !seen.insert((source_id, branch.layer, start))
        {
            return Err(
                "a generation branch needs a distinct known source, block, and start token".into(),
            );
        }
        let prefix = &report.output_token_ids[..start];
        let maximum = branch.max_new_tokens;
        if branch.clean.text != report.baseline_answer
            || branch.clean.token_ids != report.output_token_ids
            || branch.clean.stop_reason != StopReason::EndOfSequence
            || branch.clean.steps(prefix, maximum) != Some(branch.clean_patch_steps)
        {
            return Err(
                "the generation branch clean swap did not reproduce the recorded output".into(),
            );
        }
        if branch.patched.steps(prefix, maximum) != Some(branch.patched_steps)
            || branch.edited_prompt.steps(prefix, maximum).is_none()
        {
            return Err(
                "generation branches must keep the fixed prefix and record their decoding steps"
                    .into(),
            );
        }
    }
    Ok(())
}

pub(super) fn first_difference(baseline: &[u32], observed: &[u32]) -> Option<usize> {
    baseline
        .iter()
        .zip(observed)
        .position(|(a, b)| a != b)
        .or_else(|| {
            (baseline.len() != observed.len()).then_some(baseline.len().min(observed.len()))
        })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    pub location: Location,
    report_id: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    include_excerpts: bool,
}

fn default_limit() -> usize {
    12
}

pub fn query(request: &Request, reports: &[Report], allow_excerpts: bool) -> Result<Value, String> {
    if request.version != 1 || !(1..=24).contains(&request.limit) {
        return Err("Use generation branch query version 1 and a page size from 1 to 24.".into());
    }
    if request.include_excerpts && !allow_excerpts {
        return Err("This MCP process has not enabled excerpts.".into());
    }
    let (report, branches) = reports
        .iter()
        .find_map(|entry| match entry {
            Report::Timeline(report)
                if report.id == request.report_id && report.location == request.location =>
            {
                report
                    .generation_branches
                    .as_ref()
                    .map(|branches| (report, branches))
            }
            _ => None,
        })
        .ok_or("The requested generation branches are unavailable.")?;
    let states = |output: &Continuation| {
        let mut result = json!({
            "total_output_tokens": output.token_ids.len(), "stop_reason": output.stop_reason,
            "first_changed_output_token": first_difference(&report.output_token_ids, &output.token_ids),
        });
        if request.include_excerpts {
            result["text_excerpt"] = turn_evidence::excerpt(&output.text);
        }
        result
    };
    let rows: Vec<_> = branches.iter().enumerate().skip(request.offset).take(request.limit)
        .map(|(index, branch)| json!({
            "source_context_id": branch.source_context_id, "block": branch.layer,
            "start_output_token": branch.start_output_token, "max_new_tokens": branch.max_new_tokens,
            "patched_steps": branch.patched_steps, "clean_patch_steps": branch.clean_patch_steps,
            "patched": states(&branch.patched), "edited_prompt": states(&branch.edited_prompt),
            "inspector_path": activation_path(&report.location, &report.id, "branch", index, branch.layer),
        })).collect();
    let next = request.offset.saturating_add(rows.len());
    Ok(json!({
        "version": 1, "location": report.location, "report_id": report.id, "decoding": "greedy",
        "total_branches": branches.len(), "branches": rows,
        "captured_excerpts_enabled": allow_excerpts,
        "next_offset": (next < branches.len()).then_some(next),
    }))
}

pub fn available(reports: &[Report]) -> bool {
    reports.iter().any(
        |entry| matches!(entry, Report::Timeline(report) if report.generation_branches.is_some()),
    )
}

pub fn for_decision(
    reports: &[Report],
    run_id: &str,
    call_id: &str,
    decision_id: &str,
) -> Vec<Value> {
    reports.iter().filter_map(|entry| match entry {
        Report::Timeline(report) if report.location.run_id == run_id && report.location.call_id == call_id
            && report.location.decision_id == decision_id =>
            report.generation_branches.as_ref().map(|branches| json!({
                "report_id": report.id, "total_branches": branches.len(), "model_id": report.model_id,
                "runtime": report.runtime,
            })),
        _ => None,
    }).collect()
}
