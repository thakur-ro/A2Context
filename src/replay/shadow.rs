//! Compare proposals from controlled edits of a pending request.
//!
//! The runner returns a model proposal for each request. It can use an API that
//! exposes generated outputs without exposing token scores.

pub mod patterns;
mod recorded;

pub(crate) use recorded::recorded;
pub use recorded::{
    CapturedReports, Measurement, RecordedEffect, RecordedPair, RecordedProbe, Verification,
};

use super::{
    compare_control, complete_difference, complete_interaction, control_index, edited_requests,
    interval, prepare_edits, recorded_context, sample_conditions, sha256, summarize, Choice,
    ControlComparison, Intervention, MatchRule, Matcher, Pair, SamplingDesign, Summary, Trial,
    Variant,
};
use crate::{preflight, Evidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const VERSION: u32 = 1;

/// A proposal to score, declared before a runner is invoked.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub version: u32,
    pub id: String,
    pub runner_id: String,
    pub expected: Choice,
    #[serde(default)]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub sampling_design: SamplingDesign,
    pub samples: usize,
    pub interventions: Vec<Intervention>,
    #[serde(default)]
    pub test_pair: bool,
    pub negative_control_context_id: Option<String>,
}

/// Proposals, trial coverage, and changes in the declared proposal's match rate.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u32,
    pub id: String,
    pub runner_id: String,
    pub expected: Choice,
    pub match_rule: MatchRule,
    pub sampling_design: SamplingDesign,
    pub captured_request_sha256: String,
    pub baseline: Vec<Trial>,
    pub baseline_summary: Summary,
    pub effects: Vec<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<JointEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_comparison: Option<ControlComparison>,
}

/// An individual request edit and the proposal outcomes it produced.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Effect {
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub evidence: Evidence,
    pub request_sha256: String,
    #[serde(flatten)]
    pub result: Variant,
}

/// The joint edit of two declared interventions.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JointEffect {
    pub request_sha256: String,
    #[serde(flatten)]
    pub result: Pair,
}

/// Run a shadow comparison against a declared proposal on a pending request.
///
/// The runner is responsible for returning proposals before tool dispatch. All
/// request edits are verified before the first trial. A fully scored baseline
/// with zero matches is a valid baseline for a declared outcome.
///
/// # Errors
/// Returns an error if the pending request, outcome rule, or edit plan is invalid.
pub fn execute(
    input: preflight::Request,
    case: &Case,
    mut sample: impl FnMut(&Value) -> Option<Choice>,
) -> Result<Report, String> {
    if case.version != VERSION || case.id.trim().is_empty() || case.runner_id.trim().is_empty() {
        return Err("Expected a named shadow case and runner with version 1.".into());
    }
    if case.samples == 0 || case.interventions.is_empty() {
        return Err("A shadow probe requires samples and at least one source edit.".into());
    }
    if !named(&case.expected) {
        return Err("The expected proposal requires a kind and label.".into());
    }
    let match_rule = case.match_rule.normalized();
    let matcher = Matcher::new(&match_rule, &case.expected)?;
    let control = case
        .negative_control_context_id
        .as_deref()
        .map(|id| control_index(id, &case.interventions))
        .transpose()?;
    let (call, request, _) = preflight::pending_call(input)?;
    let request = request.ok_or("A shadow probe requires the captured request body.")?;
    let edits = prepare_edits(&call, &request, &case.interventions, case.test_pair)?;
    let requests = edited_requests(request, &edits, case.test_pair)?;
    let mut trials = sample_conditions(&requests, case.samples, |request| {
        sample(request).filter(named)
    });
    let joint_trials = case
        .test_pair
        .then(|| trials.pop().expect("joint condition"));
    let mut trials = trials.into_iter();
    let baseline = trials.next().expect("baseline condition");
    let baseline_summary = summarize(&baseline, &matcher);
    let variants: Vec<_> = case
        .interventions
        .iter()
        .cloned()
        .zip(edits)
        .zip(trials)
        .map(|((intervention, edit), trials)| {
            let summary = summarize(&trials, &matcher);
            let difference_pp = complete_difference(&baseline_summary, &summary);
            Variant {
                intervention,
                captured_context_sha256: edit.captured_context_sha256,
                trials,
                summary,
                difference_pp,
                difference_95_interval_pp: interval(
                    case.sampling_design,
                    difference_pp,
                    case.samples,
                    2,
                ),
            }
        })
        .collect();
    let pair = joint_trials.map(|trials| {
        let summary = summarize(&trials, &matcher);
        let difference_pp = complete_difference(&baseline_summary, &summary);
        let interaction_pp = complete_interaction(
            &baseline_summary,
            &variants[0].summary,
            &variants[1].summary,
            &summary,
        );
        JointEffect {
            request_sha256: sha256(
                requests
                    .last()
                    .expect("joint request")
                    .to_string()
                    .as_bytes(),
            ),
            result: Pair {
                trials,
                summary,
                difference_pp,
                interaction_pp,
                difference_95_interval_pp: interval(
                    case.sampling_design,
                    difference_pp,
                    case.samples,
                    2,
                ),
                interaction_95_interval_pp: interval(
                    case.sampling_design,
                    interaction_pp,
                    case.samples,
                    4,
                ),
            },
        }
    });
    let control_comparison = control
        .map(|index| compare_control(&variants[index], &variants[1 - index], case.sampling_design));
    let effects = variants
        .into_iter()
        .zip(&requests[1..])
        .map(|(result, request)| {
            let (item, _) = recorded_context(&call, &result.intervention.context_id)?;
            Ok(Effect {
                source_id: item.source_id.clone(),
                source_version: item.source_version.clone(),
                evidence: item.evidence.clone(),
                request_sha256: sha256(request.to_string().as_bytes()),
                result,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Report {
        version: VERSION,
        id: case.id.clone(),
        runner_id: case.runner_id.clone(),
        expected: case.expected.clone(),
        match_rule,
        sampling_design: case.sampling_design,
        captured_request_sha256: sha256(
            call.recorded_request
                .as_ref()
                .expect("captured body")
                .body
                .as_bytes(),
        ),
        baseline,
        baseline_summary,
        effects,
        pair,
        control_comparison,
    })
}

fn named(choice: &Choice) -> bool {
    !choice.kind.trim().is_empty() && !choice.label.trim().is_empty()
}
