//! Align teacher-forced token scores with independently verified proposal trials.
use super::{ConditionScore, Measurement, TokenStep};
use crate::replay::{shadow, MatchRule};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TargetPath<'a> {
    pub report: &'a str,
    pub model: &'a str,
    pub revision: &'a str,
    pub runtime: &'a str,
    pub assistant_prefix: &'a str,
    pub continuation: &'a str,
    pub token_ids: &'a [u32],
    pub conditions: Vec<TargetCondition<'a>>,
}

#[derive(Debug, Serialize)]
pub struct TargetCondition<'a> {
    pub label: &'a str,
    pub steps: &'a [TokenStep],
    pub prefix_probability: f64,
    /// Zero-based position where another token strictly outscored the supplied target.
    pub first_lower: Option<usize>,
}

fn condition<'a>(
    label: &'a str,
    score: &'a ConditionScore,
    index: usize,
) -> Option<TargetCondition<'a>> {
    let steps = &score.steps.as_ref()?[index];
    Some(TargetCondition {
        label,
        steps,
        prefix_probability: score.log_probs[index].exp(),
        first_lower: steps
            .iter()
            .position(|step| step.log_prob < step.top_log_prob),
    })
}

/// Join an exact text target to scores starting at the answer boundary. A single
/// edit must resolve to the same source and edited request in both reports.
/// Scores describe token prefixes. Trials apply the match rule to complete outputs.
pub fn target_path<'a>(
    candidate: &'a Measurement,
    shadow: &'a shadow::Measurement,
) -> Option<TargetPath<'a>> {
    let probe = candidate.report;
    let trials = shadow.report;
    if !matches!(trials.match_rule, MatchRule::Exact)
        || trials.expected.kind != "model_output"
        || !probe.assistant_prefix.is_empty()
        || shadow.model != Some(candidate.model_id)
        || probe.captured_request_sha256 != trials.captured_request_sha256
    {
        return None;
    }
    let expected = trials.expected.outcome.as_str()?;
    let index = probe
        .candidates
        .iter()
        .position(|c| c.continuation == expected)?;
    let scores = &probe.scores.conditions;
    let mut conditions = vec![condition("Original", &scores[0], index)?];
    for (bound, raw) in shadow.effects.iter().zip(&trials.effects) {
        let slot = candidate
            .effects
            .iter()
            .zip(&probe.effects)
            .position(|(other, edit)| {
                bound.context_id == other.context_id
                    && bound.source_id == other.source_id
                    && bound.source_version == other.source_version
                    && raw.request_sha256 == edit.request_sha256
                    && other.pointer == bound.intervention.pointer
                    && other.text_range == bound.intervention.text_range
                    && other.operation == bound.intervention.operation
                    && other.replacement == bound.intervention.replacement
            });
        if let Some(score) = slot.and_then(|i| {
            condition(
                bound.source_id.unwrap_or(bound.context_id),
                &scores[i + 1],
                index,
            )
        }) {
            conditions.push(score);
        }
    }
    if conditions.len() == 1 {
        return None;
    }
    if conditions.len() == 3 && joint_matches(probe, trials) {
        conditions.extend(condition("Both edits", scores.last()?, index));
    }
    Some(path(candidate, index, conditions))
}

fn path<'a>(
    candidate: &'a Measurement,
    index: usize,
    conditions: Vec<TargetCondition<'a>>,
) -> TargetPath<'a> {
    TargetPath {
        report: &candidate.report.id,
        model: candidate.model_id,
        revision: candidate.model_revision,
        runtime: candidate.runtime,
        assistant_prefix: candidate.assistant_prefix,
        continuation: &candidate.candidates[index].continuation,
        token_ids: &candidate.scores.candidate_token_ids[index],
        conditions,
    }
}

/// Read token steps for either supplied candidate before any proposal trial is available.
pub fn continuation_path<'a>(candidate: &'a Measurement, index: usize) -> Option<TargetPath<'a>> {
    candidate.candidates.get(index)?;
    let scores = &candidate.scores.conditions;
    let mut conditions = vec![condition("Original", &scores[0], index)?];
    for (effect, row) in candidate.effects.iter().zip(&scores[1..]) {
        conditions.extend(condition(
            effect.source_id.unwrap_or(effect.context_id),
            row,
            index,
        ));
    }
    if candidate.report.joint_request_sha256.is_some() {
        conditions.extend(condition("Both edits", scores.last()?, index));
    }
    Some(path(candidate, index, conditions))
}

pub(super) fn joint_matches(probe: &super::Report, trials: &shadow::Report) -> bool {
    probe
        .joint_request_sha256
        .as_ref()
        .zip(trials.pair.as_ref())
        .is_some_and(|(hash, pair)| hash == &pair.request_sha256)
}
