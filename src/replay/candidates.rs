//! Compare two fixed candidate continuations before a workflow commits a decision.
//!
//! Build the baseline and source edits from a pending request. A runtime adapter
//! scores both candidates at the same assistant prefix for every condition.

use super::{prepare_edit, sha256, Intervention};
pub mod patterns;
mod recorded;

pub(crate) use recorded::recorded;
pub use recorded::{CapturedReports, Measurement, RecordedEffect, RecordedProbe, Verification};

use crate::{preflight, Evidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const VERSION: u32 = 1;

/// The candidates and controlled edits to score on a pending request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub version: u32,
    pub id: String,
    pub scorer_id: String,
    #[serde(default)]
    pub assistant_prefix: String,
    pub candidates: [Candidate; 2],
    pub interventions: Vec<Intervention>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    /// The exact continuation, including any space or punctuation at its boundary.
    pub continuation: String,
}

/// One batch sent to a runtime adapter. Conditions are ordered baseline first.
#[derive(Debug, Serialize)]
pub struct ScoringRequest<'a> {
    pub version: u32,
    pub assistant_prefix: &'a str,
    pub candidates: &'a [Candidate; 2],
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Serialize)]
pub struct Condition {
    pub id: String,
    pub request: Value,
}

/// A runtime's scores for the full batch, using one model and tokenizer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoringResult {
    pub version: u32,
    pub model_id: String,
    pub model_revision: String,
    pub runtime: String,
    pub tokenizer_sha256: String,
    pub assistant_prefix_token_ids: Vec<u32>,
    pub candidate_token_ids: [Vec<u32>; 2],
    pub conditions: Vec<ConditionScore>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionScore {
    pub id: String,
    /// Hash of the rendered prompt and fixed assistant prefix for this condition.
    pub rendered_prefix_sha256: String,
    /// Natural log probabilities of the two complete candidate token sequences.
    pub log_probs: [f64; 2],
}

/// Conditional preference shifts and the exact source edits used to measure them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u32,
    pub id: String,
    pub scorer_id: String,
    pub captured_request_sha256: String,
    pub assistant_prefix: String,
    pub candidates: [Candidate; 2],
    pub scores: ScoringResult,
    pub baseline_log_odds: f64,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub condition_id: String,
    pub intervention: Intervention,
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub evidence: Evidence,
    pub request_sha256: String,
    pub log_odds: f64,
    pub change_log_odds: f64,
}

/// Score a pending request and exact source edits against two fixed alternatives.
///
/// The scorer is called once after all edits have been checked against captured
/// context. Positive margins favor the first candidate. A change is the edited
/// margin minus the baseline margin.
///
/// # Errors
///
/// Returns an error if the request, source edits, or runtime scores fail validation.
pub fn execute(
    input: preflight::Request,
    case: &Case,
    score: impl FnOnce(&ScoringRequest<'_>) -> Result<ScoringResult, String>,
) -> Result<Report, String> {
    validate_case(
        case.version,
        &case.id,
        &case.scorer_id,
        &case.candidates,
        case.interventions.len(),
    )?;
    let (call, request, _) = preflight::pending_call(input)?;
    let request = request.ok_or("A candidate probe requires the captured request body.")?;
    let mut seen = BTreeSet::new();
    let mut conditions = vec![Condition {
        id: "baseline".into(),
        request,
    }];
    let mut sources = Vec::new();
    for intervention in &case.interventions {
        if !seen.insert(&intervention.context_id) {
            return Err(format!(
                "Duplicate intervention for {}",
                intervention.context_id
            ));
        }
        let (item, _) = super::recorded_context(&call, &intervention.context_id)?;
        let mut request = conditions[0].request.clone();
        prepare_edit(&call, intervention)?.apply(&mut request)?;
        conditions.push(Condition {
            id: format!("edit:{}", item.id),
            request,
        });
        sources.push(item);
    }

    let work = ScoringRequest {
        version: VERSION,
        assistant_prefix: &case.assistant_prefix,
        candidates: &case.candidates,
        conditions,
    };
    let scores = score(&work)?;
    validate_scores(
        work.assistant_prefix,
        work.conditions.iter().map(|row| row.id.as_str()),
        &scores,
    )?;
    let baseline_log_odds = margin(&scores.conditions[0]);
    let mut effects = Vec::new();
    for (((intervention, item), condition), score) in case
        .interventions
        .iter()
        .zip(sources)
        .zip(&work.conditions[1..])
        .zip(&scores.conditions[1..])
    {
        let log_odds = margin(score);
        let change_log_odds = log_odds - baseline_log_odds;
        if !change_log_odds.is_finite() {
            return Err("A change in log odds is outside the supported numeric range.".into());
        }
        effects.push(Effect {
            condition_id: condition.id.clone(),
            intervention: intervention.clone(),
            source_id: item.source_id.clone(),
            source_version: item.source_version.clone(),
            evidence: item.evidence.clone(),
            request_sha256: sha256(condition.request.to_string().as_bytes()),
            log_odds,
            change_log_odds,
        });
    }
    Ok(Report {
        version: VERSION,
        id: case.id.clone(),
        scorer_id: case.scorer_id.clone(),
        captured_request_sha256: sha256(
            call.recorded_request
                .as_ref()
                .expect("captured body")
                .body
                .as_bytes(),
        ),
        assistant_prefix: case.assistant_prefix.clone(),
        candidates: case.candidates.clone(),
        scores,
        baseline_log_odds,
        effects,
    })
}

fn validate_case(
    version: u32,
    id: &str,
    scorer_id: &str,
    candidates: &[Candidate; 2],
    edits: usize,
) -> Result<(), String> {
    let [first, second] = candidates;
    if version != VERSION || id.trim().is_empty() || scorer_id.trim().is_empty() {
        return Err("Expected a named candidate probe and scorer with version 1.".into());
    }
    if first.id.trim().is_empty()
        || second.id.trim().is_empty()
        || first.id == second.id
        || !disjoint(
            first.continuation.as_bytes(),
            second.continuation.as_bytes(),
        )
    {
        return Err("Choose distinct candidate IDs and two non-overlapping continuations.".into());
    }
    if edits == 0 {
        return Err("A candidate probe requires at least one source edit.".into());
    }
    Ok(())
}

fn disjoint<T: PartialEq>(first: &[T], second: &[T]) -> bool {
    !first.starts_with(second) && !second.starts_with(first)
}

fn validate_scores<'a>(
    prefix: &str,
    conditions: impl IntoIterator<Item = &'a str>,
    scores: &ScoringResult,
) -> Result<(), String> {
    let digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if scores.version != VERSION
        || scores.model_id.trim().is_empty()
        || scores.model_revision.trim().is_empty()
        || scores.runtime.trim().is_empty()
        || !digest(&scores.tokenizer_sha256)
        || !disjoint(
            &scores.candidate_token_ids[0],
            &scores.candidate_token_ids[1],
        )
        || scores.assistant_prefix_token_ids.is_empty() != prefix.is_empty()
    {
        return Err("The scorer must identify its runtime and return disjoint candidate tokens at the fixed prefix.".into());
    }
    let missing_score = "The scorer must return one score for every requested condition.";
    let mut returned = scores.conditions.iter();
    for id in conditions {
        let score = returned.next().ok_or(missing_score)?;
        let [first, second] = score.log_probs;
        if id != score.id
            || !digest(&score.rendered_prefix_sha256)
            || !score
                .log_probs
                .iter()
                .all(|value| value.is_finite() && *value <= 0.0)
            || first.exp() + second.exp() > 1.0 + 1e-5
        {
            return Err(format!("Invalid or mismatched candidate scores for {id}"));
        }
    }
    if returned.next().is_some() {
        return Err(missing_score.into());
    }
    Ok(())
}

fn margin(score: &ConditionScore) -> f64 {
    score.log_probs[0] - score.log_probs[1]
}
