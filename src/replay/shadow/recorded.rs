//! Match a shadow report to the recorded request and recompute its comparisons.

use super::super::{
    compare_control, complete_difference, complete_interaction, control_index, edited_requests,
    interval, prepare_edits, sha256, source_binding, summarize, Choice, ControlComparison,
    Intervention, MatchRule, Matcher, ReplayInterval, SamplingDesign, Summary, Trial,
};
use super::{named, Report, VERSION};
use crate::{Call, ContextItem, Evidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Shadow reports and the trace attribute that carried them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CapturedReports {
    pub evidence: Evidence,
    pub reports: Option<Vec<Report>>,
}

/// A shadow attachment and the result of matching it to one inference call.
#[derive(Debug, Serialize)]
pub struct RecordedProbe<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub evidence: &'a Evidence,
    pub report_id: Option<&'a str>,
    #[serde(flatten)]
    pub verification: Verification<'a>,
}

/// A match covers the request, its source edits, and arithmetic over supplied
/// trials. The workflow supplies proposal authenticity and dispatch timing.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verification<'a> {
    Matched { measurement: Measurement<'a> },
    Unverified { issue: String },
}

/// Verified comparisons for a declared proposal. Raw trials stay in the report.
#[derive(Debug, Serialize)]
pub struct Measurement<'a> {
    pub runner_id: &'a str,
    pub expected: &'a Choice,
    pub match_rule: &'a MatchRule,
    pub sampling_design: SamplingDesign,
    pub baseline: &'a Summary,
    pub effects: Vec<RecordedEffect<'a>>,
    pub pair: Option<RecordedPair<'a>>,
    pub control_comparison: Option<&'a ControlComparison>,
}

/// One effect resolved to its source on the recorded call.
#[derive(Debug, Serialize)]
pub struct RecordedEffect<'a> {
    #[serde(skip)]
    pub(super) context: &'a ContextItem,
    #[serde(skip)]
    pub(super) captured_context_sha256: &'a str,
    pub context_id: &'a str,
    pub source_id: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub evidence: &'a Evidence,
    pub intervention: &'a Intervention,
    pub summary: &'a Summary,
    pub difference_pp: Option<f64>,
    pub difference_95_interval_pp: Option<ReplayInterval>,
}

/// The measured proposal rate and interaction for a joint edit.
#[derive(Debug, Serialize)]
pub struct RecordedPair<'a> {
    pub summary: &'a Summary,
    pub difference_pp: Option<f64>,
    pub interaction_pp: Option<f64>,
    pub difference_95_interval_pp: Option<ReplayInterval>,
    pub interaction_95_interval_pp: Option<ReplayInterval>,
}

/// Verify captured probes on one call against their request and source edits.
pub fn recorded<'a>(run_id: &'a str, call: &'a Call) -> Vec<RecordedProbe<'a>> {
    let Some(capture) = &call.shadow_probes else {
        return Vec::new();
    };
    let at_call = |report_id, verification| RecordedProbe {
        run_id,
        call_id: &call.id,
        evidence: &capture.evidence,
        report_id,
        verification,
    };
    let Some(reports) = &capture.reports else {
        return vec![at_call(
            None,
            Verification::Unverified {
                issue: "The workflow supplied unreadable shadow probe reports.".into(),
            },
        )];
    };
    let mut seen = BTreeSet::new();
    let duplicates: BTreeSet<_> = reports
        .iter()
        .filter_map(|report| (!seen.insert(&report.id)).then_some(&report.id))
        .collect();
    let request = call
        .recorded_request
        .as_ref()
        .and_then(|body| Some((body.parse().ok()?, sha256(body.body.as_bytes()))));
    reports
        .iter()
        .map(|report| {
            let result = if duplicates.contains(&report.id) {
                Err("Multiple shadow probe reports use this ID on the same call.".into())
            } else if let Some((request, hash)) = &request {
                verify(call, request, hash, report)
            } else {
                Err(
                    "The original request body was unavailable for shadow probe verification."
                        .into(),
                )
            };
            let verification = match result {
                Ok(measurement) => Verification::Matched { measurement },
                Err(issue) => Verification::Unverified { issue },
            };
            at_call(Some(report.id.as_str()), verification)
        })
        .collect()
}

fn verify<'a>(
    call: &'a Call,
    request: &Value,
    hash: &str,
    report: &'a Report,
) -> Result<Measurement<'a>, String> {
    let samples = report.baseline.len();
    if report.version != VERSION
        || report.id.trim().is_empty()
        || report.runner_id.trim().is_empty()
        || !named(&report.expected)
        || samples == 0
        || report.effects.is_empty()
    {
        return Err("Invalid shadow report version, identities, or trial plan.".into());
    }
    if report.captured_request_sha256 != hash {
        return Err("The recorded request differs from the shadow probe request.".into());
    }
    let matcher = Matcher::new(&report.match_rule, &report.expected)
        .map_err(|_| "The shadow report has an invalid proposal match rule.")?;
    let check_trials = |trials: &[Trial], summary: &Summary| {
        trials.len() == samples
            && trials.iter().all(|trial| match trial {
                Trial::Completed { choice } => named(choice),
                Trial::Failed => true,
            })
            && summarize(trials, &matcher) == *summary
    };
    if !check_trials(&report.baseline, &report.baseline_summary) {
        return Err("The shadow baseline counts differ from its proposals.".into());
    }
    let sources = report
        .effects
        .iter()
        .map(|effect| {
            source_binding::resolve(
                call,
                &effect.result.intervention,
                effect.source_id.as_deref(),
                effect.source_version.as_deref(),
            )
            .ok_or_else(|| {
                format!(
                    "The shadow source {} does not identify one captured context entry.",
                    effect.result.intervention.context_id
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let interventions: Vec<_> = report
        .effects
        .iter()
        .zip(&sources)
        .map(|(effect, source)| {
            let mut intervention = effect.result.intervention.clone();
            intervention.context_id.clone_from(&source.id);
            intervention
        })
        .collect();
    let test_pair = report.pair.is_some();
    let edits = prepare_edits(call, request, &interventions, test_pair)?;
    let requests = edited_requests(request.clone(), &edits, test_pair)?;
    let baseline = &report.baseline_summary;
    let interval_for =
        |difference, conditions| interval(report.sampling_design, difference, samples, conditions);
    let mut effects = Vec::with_capacity(sources.len());
    for (index, (effect, source)) in report.effects.iter().zip(sources).enumerate() {
        let variant = &effect.result;
        if variant.captured_context_sha256 != edits[index].captured_context_sha256
            || effect.request_sha256 != sha256(requests[index + 1].to_string().as_bytes())
        {
            return Err(format!(
                "The edited request for {} differs from the shadow probe.",
                source.id
            ));
        }
        let difference = complete_difference(baseline, &variant.summary);
        if !check_trials(&variant.trials, &variant.summary)
            || !same_contrast(variant.difference_pp, difference)
            || variant.difference_95_interval_pp != interval_for(difference, 2)
        {
            return Err(format!(
                "The shadow effect for {} differs from its proposal trials.",
                source.id
            ));
        }
        effects.push(RecordedEffect {
            context: source,
            captured_context_sha256: &variant.captured_context_sha256,
            context_id: &source.id,
            source_id: source.source_id.as_deref(),
            source_version: source.source_version.as_deref(),
            evidence: &source.evidence,
            intervention: &variant.intervention,
            summary: &variant.summary,
            difference_pp: difference,
            difference_95_interval_pp: variant.difference_95_interval_pp,
        });
    }
    let pair = report
        .pair
        .as_ref()
        .map(|joint| {
            let pair = &joint.result;
            let difference = complete_difference(baseline, &pair.summary);
            let interaction = complete_interaction(
                baseline,
                effects[0].summary,
                effects[1].summary,
                &pair.summary,
            );
            if joint.request_sha256 != sha256(requests[3].to_string().as_bytes())
                || !check_trials(&pair.trials, &pair.summary)
                || !same_contrast(pair.difference_pp, difference)
                || !same_contrast(pair.interaction_pp, interaction)
                || pair.difference_95_interval_pp != interval_for(difference, 2)
                || pair.interaction_95_interval_pp != interval_for(interaction, 4)
            {
                return Err(
                    "The shadow joint request or comparison differs from its proposal trials.",
                );
            }
            Ok(RecordedPair {
                summary: &pair.summary,
                difference_pp: difference,
                interaction_pp: interaction,
                difference_95_interval_pp: pair.difference_95_interval_pp,
                interaction_95_interval_pp: pair.interaction_95_interval_pp,
            })
        })
        .transpose()?;
    if let Some(control) = &report.control_comparison {
        let index = control_index(
            &control.control_context_id,
            report
                .effects
                .iter()
                .map(|effect| &effect.result.intervention),
        )?;
        let computed = compare_control(
            &report.effects[index].result,
            &report.effects[1 - index].result,
            report.sampling_design,
        );
        if !same_contrast(
            control.target_minus_control_pp,
            computed.target_minus_control_pp,
        ) || control.target_minus_control_95_interval_pp
            != computed.target_minus_control_95_interval_pp
        {
            return Err("The shadow control comparison differs from its proposal trials.".into());
        }
    }
    Ok(Measurement {
        runner_id: &report.runner_id,
        expected: &report.expected,
        match_rule: &report.match_rule,
        sampling_design: report.sampling_design,
        baseline,
        effects,
        pair,
        control_comparison: report.control_comparison.as_ref(),
    })
}

fn same_contrast(reported: Option<f64>, computed: Option<f64>) -> bool {
    match (reported, computed) {
        (Some(reported), Some(computed)) => (reported - computed).abs() <= 1e-10,
        (None, None) => true,
        _ => false,
    }
}
