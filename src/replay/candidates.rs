//! Compare two fixed candidate continuations before a workflow commits a decision.
//!
//! Build the baseline and source edits from a pending request. A runtime adapter
//! scores both candidates at the same assistant prefix for every condition.

use super::{edited_requests, prepare_edits, sha256, Intervention};
pub mod patterns;
mod recorded;
mod target;
pub use target::{continuation_path, target_path, TargetCondition, TargetPath};

pub use recorded::recorded;
pub use recorded::{CapturedReports, Measurement, RecordedEffect, RecordedProbe, Verification};

use crate::{preflight, Evidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(default)]
    pub test_pair: bool,
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
    /// Optional vocabulary readouts, in token order along each supplied continuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<[Vec<TokenStep>; 2]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenStep {
    pub text: String,
    pub log_prob: f64,
    pub top_id: u32,
    pub top_text: String,
    pub top_log_prob: f64,
}

impl ConditionScore {
    /// Remaining probability after the two disjoint token prefixes at this condition.
    pub fn outside_pair_probability(&self) -> f64 {
        (1.0 - self.log_probs[0].exp() - self.log_probs[1].exp()).clamp(0.0, 1.0)
    }
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
    /// When present, the final score belongs to both edits applied together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joint_request_sha256: Option<String>,
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
    let edits = prepare_edits(&call, &request, &case.interventions, case.test_pair)?;
    let requests = edited_requests(request, &edits, case.test_pair)?;
    let sources = case
        .interventions
        .iter()
        .map(|i| super::recorded_context(&call, &i.context_id).map(|(item, _)| item))
        .collect::<Result<Vec<_>, _>>()?;
    let ids = std::iter::once("baseline".to_owned())
        .chain(sources.iter().map(|item| format!("edit:{}", item.id)))
        .chain(case.test_pair.then(|| "joint".to_owned()));
    let conditions = ids
        .zip(requests)
        .map(|(id, request)| Condition { id, request })
        .collect();

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
    let joint_request_sha256 = case.test_pair.then(|| {
        sha256(
            work.conditions
                .last()
                .expect("joint condition")
                .request
                .to_string()
                .as_bytes(),
        )
    });
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
        joint_request_sha256,
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
            || !valid_steps(score, &scores.candidate_token_ids)
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

fn valid_steps(score: &ConditionScore, tokens: &[Vec<u32>; 2]) -> bool {
    let Some(steps) = &score.steps else {
        return true;
    };
    for ((path, ids), sequence_score) in steps.iter().zip(tokens).zip(score.log_probs) {
        if path.len() != ids.len() {
            return false;
        }
        let mut sum = 0.0;
        for (step, id) in path.iter().zip(ids) {
            if !step.log_prob.is_finite()
                || !step.top_log_prob.is_finite()
                || step.top_log_prob > 0.0
                || step.log_prob > step.top_log_prob
            {
                return false;
            }
            if step.top_id == *id {
                if step.log_prob != step.top_log_prob || step.text != step.top_text {
                    return false;
                }
            } else if step.log_prob.exp() + step.top_log_prob.exp() > 1.0 + 1e-5 {
                return false;
            }
            sum += step.log_prob;
        }
        if (sum - sequence_score).abs() > 1e-9 * (1.0 + sequence_score.abs()) {
            return false;
        }
    }
    // Both alternatives start at the same prefix. Their vocabulary maxima
    // must agree until the supplied token sequences diverge. Separate forward
    // passes can round differently for continuations of different lengths.
    for (i, (first, second)) in steps[0].iter().zip(&steps[1]).enumerate() {
        if (first.top_log_prob - second.top_log_prob).abs() > 1e-4
            || (first.top_id == second.top_id && first.top_text != second.top_text)
        {
            return false;
        }
        if tokens[0][i] != tokens[1][i] {
            break;
        }
        if first.text != second.text || (first.log_prob - second.log_prob).abs() > 1e-4 {
            return false;
        }
    }
    true
}

fn margin(score: &ConditionScore) -> f64 {
    score.log_probs[0] - score.log_probs[1]
}
