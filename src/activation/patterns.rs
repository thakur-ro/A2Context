//! Keep source interventions comparable across recorded conversations.

pub mod query;

use super::{Report, Target};
use crate::{replay::Location, Run};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct Group<'a> {
    #[serde(flatten)]
    pub comparison: Comparison<'a>,
    pub occurrences: Vec<Occurrence<'a>>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Comparison<'a> {
    model_id: &'a str,
    model_revision: &'a str,
    runtime: &'a str,
    tokenizer_sha256: &'a str,
    layer_count: usize,
    decision_kind: &'a str,
    decision_label: &'a str,
    source_id: Option<&'a str>,
    source_version: Option<&'a str>,
    context_kind: &'a str,
    context_role: Option<&'a str>,
    captured_text_sha256: &'a str,
    edited_range_in_source: [usize; 2],
    before: &'a str,
    after: &'a str,
    checkpoint: Checkpoint<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Checkpoint<'a> {
    label: &'a str,
    output_token: usize,
    alternatives: &'a [String; 2],
    choice_token_ids: [u32; 2],
    fixed_prefix_token_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Occurrence<'a> {
    pub location: &'a Location,
    pub report_id: &'a str,
    pub context_id: &'a str,
    pub trace_source: &'a str,
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub checkpoint_index: usize,
    pub baseline_margin: f64,
    pub patched_margins: &'a [f64],
}

/// Group validated activation timelines for the loaded runs.
pub fn group<'a>(runs: &[&'a Run], reports: &'a [Report]) -> Vec<Group<'a>> {
    let by_id: BTreeMap<_, _> = runs.iter().map(|run| (run.id.as_str(), *run)).collect();
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for report in reports {
        let Report::Timeline(report) = report else {
            continue;
        };
        let location = &report.location;
        let Some(run) = by_id.get(location.run_id.as_str()) else {
            continue;
        };
        let Some(call) = run.calls.iter().find(|call| call.id == location.call_id) else {
            continue;
        };
        let Some(decision) = call
            .decisions
            .iter()
            .find(|item| item.id == location.decision_id)
        else {
            continue;
        };
        for (checkpoint_index, step) in report.checkpoints.iter().enumerate() {
            let Some(prefix) = report.output_token_ids.get(..step.output_token) else {
                continue;
            };
            let mut digest = Sha256::new();
            for token in prefix {
                digest.update(token.to_be_bytes());
            }
            let checkpoint = Checkpoint {
                label: &step.label,
                output_token: step.output_token,
                alternatives: &step.alternatives,
                choice_token_ids: step.choice_token_ids,
                fixed_prefix_token_sha256: format!("{:x}", digest.finalize()),
            };
            for source in &report.sources {
                let Some(context) = call
                    .context
                    .iter()
                    .find(|entry| entry.id == source.context_id)
                else {
                    continue;
                };
                let Some(sweep) = step.sweeps.iter().find(|sweep| {
                    sweep.target == Target::SourceTokens
                        && sweep.source_context_id == source.context_id
                }) else {
                    continue;
                };
                let [start, end] = source.edited_token_range;
                let offset = source.token_range[0];
                let (Some(start), Some(end)) = (start.checked_sub(offset), end.checked_sub(offset))
                else {
                    continue;
                };
                let comparison = Comparison {
                    model_id: &report.model_id,
                    model_revision: &report.model_revision,
                    runtime: &report.runtime,
                    tokenizer_sha256: &report.tokenizer_sha256,
                    layer_count: report.layer_count,
                    decision_kind: &decision.kind,
                    decision_label: &decision.label,
                    source_id: context.source_id.as_deref(),
                    source_version: context.source_version.as_deref(),
                    context_kind: &context.kind,
                    context_role: context.role.as_deref(),
                    captured_text_sha256: &source.captured_text_sha256,
                    edited_range_in_source: [start, end],
                    before: &source.before,
                    after: &source.after,
                    checkpoint: checkpoint.clone(),
                };
                groups.entry(comparison).or_default().push(Occurrence {
                    location,
                    report_id: &report.id,
                    context_id: &source.context_id,
                    trace_source: &run.source,
                    organization_id: run.organization_id.as_deref(),
                    user_id: run.user_id.as_deref(),
                    conversation_id: run.conversation_id.as_deref(),
                    turn_id: call.turn_id.as_deref(),
                    started_at: call.started_at.as_deref(),
                    checkpoint_index,
                    baseline_margin: step.baseline_margin,
                    patched_margins: &sweep.margins,
                });
            }
        }
    }
    groups
        .into_iter()
        .map(|(comparison, occurrences)| Group {
            comparison,
            occurrences,
        })
        .collect()
}
