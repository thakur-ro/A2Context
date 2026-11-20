//! Bind reported candidate measurements to the exact recorded request and sources.

use super::{margin, sha256, validate_case, validate_scores, Candidate, Report, ScoringResult};
use crate::{
    replay::{edited_requests, prepare_edit, prepare_edits, source_binding, Operation},
    Call, ContextItem, Evidence, TextRange,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Candidate probe reports and the trace attribute that carried them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CapturedReports {
    pub evidence: Evidence,
    pub reports: Option<Vec<Report>>,
}

/// A trace attachment and the result of matching it to a recorded call.
#[derive(Debug, Serialize)]
pub struct RecordedProbe<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub evidence: &'a Evidence,
    pub report_id: Option<&'a str>,
    #[serde(flatten)]
    pub verification: Verification<'a>,
}

/// A match verifies request identity, source edits, and score arithmetic.
/// The scoring runtime supplies the model probabilities.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verification<'a> {
    Matched { measurement: Measurement<'a> },
    Unverified { issue: String },
}

/// The reported model comparison resolved to captured sources on this call.
#[derive(Debug, Serialize)]
pub struct Measurement<'a> {
    #[serde(skip)]
    pub(in crate::replay) report: &'a Report,
    #[serde(skip)]
    pub(super) scores: &'a ScoringResult,
    pub scorer_id: &'a str,
    pub model_id: &'a str,
    pub model_revision: &'a str,
    pub runtime: &'a str,
    #[serde(skip)]
    pub tokenizer_sha256: &'a str,
    pub assistant_prefix: &'a str,
    pub candidates: &'a [Candidate; 2],
    pub baseline_log_odds: f64,
    pub baseline_outside_pair_probability: f64,
    pub effects: Vec<RecordedEffect<'a>>,
}

/// The preference shift and exact request location of one source edit.
#[derive(Debug, Serialize)]
pub struct RecordedEffect<'a> {
    #[serde(skip)]
    pub(super) context: &'a ContextItem,
    #[serde(skip)]
    pub(super) captured_context_sha256: String,
    pub context_id: &'a str,
    pub source_id: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub evidence: &'a Evidence,
    pub pointer: &'a str,
    pub text_range: Option<TextRange>,
    pub operation: Operation,
    pub replacement: &'a str,
    pub log_odds: f64,
    pub change_log_odds: f64,
    pub outside_pair_probability: f64,
}

/// Verify captured probes on one call against their request and source edits.
pub fn recorded<'a>(run_id: &'a str, call: &'a Call) -> Vec<RecordedProbe<'a>> {
    let Some(capture) = &call.candidate_probes else {
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
                issue: "The workflow supplied unreadable candidate probe reports.".into(),
            },
        )];
    };
    let mut seen = BTreeSet::new();
    let duplicates = reports
        .iter()
        .filter_map(|report| (!seen.insert(report.id.as_str())).then_some(report.id.as_str()))
        .collect::<BTreeSet<_>>();
    let request = call
        .recorded_request
        .as_ref()
        .and_then(|body| Some((body.parse().ok()?, sha256(body.body.as_bytes()))));
    reports
        .iter()
        .map(|report| {
            let result = if duplicates.contains(report.id.as_str()) {
                Err("Multiple candidate probe reports use this ID on the same call.".into())
            } else if let Some((request, hash)) = &request {
                verify(call, request, hash, report)
            } else {
                Err(
                    "The original request body was unavailable for candidate probe verification."
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

pub(super) fn verify<'a>(
    call: &'a Call,
    request: &Value,
    hash: &str,
    report: &'a Report,
) -> Result<Measurement<'a>, String> {
    validate_case(
        report.version,
        &report.id,
        &report.scorer_id,
        &report.candidates,
        report.effects.len(),
    )?;
    if report.captured_request_sha256 != hash {
        return Err("The recorded request differs from the candidate probe request.".into());
    }
    let expected = std::iter::once("baseline").chain(
        report
            .effects
            .iter()
            .map(|effect| effect.condition_id.as_str()),
    );
    let expected = expected.chain(report.joint_request_sha256.as_ref().map(|_| "joint"));
    validate_scores(&report.assistant_prefix, expected, &report.scores)?;
    let baseline = margin(&report.scores.conditions[0]);
    if !same_score(baseline, report.baseline_log_odds) {
        return Err("The reported baseline margin differs from its candidate scores.".into());
    }

    let mut seen = BTreeSet::new();
    let mut effects = Vec::new();
    for (effect, score) in report.effects.iter().zip(&report.scores.conditions[1..]) {
        if effect.condition_id != format!("edit:{}", effect.intervention.context_id) {
            return Err("A candidate probe condition does not identify its source edit.".into());
        }
        let item = source_binding::resolve(
            call,
            &effect.intervention,
            effect.source_id.as_deref(),
            effect.source_version.as_deref(),
        )
        .ok_or_else(|| {
            format!(
                "The probe source {} does not identify one captured context entry.",
                effect.intervention.context_id
            )
        })?;
        if !seen.insert(item.id.as_str()) {
            return Err("A captured source appears in more than one edit in this probe.".into());
        }
        let mut edited = request.clone();
        let mut intervention = effect.intervention.clone();
        intervention.context_id.clone_from(&item.id);
        let prepared = prepare_edit(call, &intervention)?;
        prepared.apply(&mut edited)?;
        if sha256(edited.to_string().as_bytes()) != effect.request_sha256 {
            return Err(format!(
                "The edited request for {} differs from the candidate probe.",
                item.id
            ));
        }
        let log_odds = margin(score);
        let change_log_odds = log_odds - baseline;
        if !same_score(log_odds, effect.log_odds)
            || !same_score(change_log_odds, effect.change_log_odds)
        {
            return Err(format!(
                "The reported effect for {} differs from its candidate scores.",
                item.id
            ));
        }
        effects.push(RecordedEffect {
            context: item,
            captured_context_sha256: prepared.captured_context_sha256,
            context_id: &item.id,
            source_id: item.source_id.as_deref(),
            source_version: item.source_version.as_deref(),
            evidence: &item.evidence,
            pointer: &effect.intervention.pointer,
            text_range: effect.intervention.text_range,
            operation: effect.intervention.operation,
            replacement: &effect.intervention.replacement,
            log_odds,
            change_log_odds,
            outside_pair_probability: score.outside_pair_probability(),
        });
    }

    if let Some(hash) = &report.joint_request_sha256 {
        let edits: Vec<_> = report
            .effects
            .iter()
            .zip(&effects)
            .map(|(raw, bound)| {
                let mut edit = raw.intervention.clone();
                edit.context_id = bound.context_id.to_owned();
                edit
            })
            .collect();
        let prepared = prepare_edits(call, request, &edits, true)?;
        let requests = edited_requests(request.clone(), &prepared, true)?;
        if sha256(
            requests
                .last()
                .expect("joint condition")
                .to_string()
                .as_bytes(),
        ) != *hash
        {
            return Err("The combined edits differ from the candidate probe request.".into());
        }
    }
    Ok(Measurement {
        report,
        scores: &report.scores,
        scorer_id: &report.scorer_id,
        model_id: &report.scores.model_id,
        model_revision: &report.scores.model_revision,
        runtime: &report.scores.runtime,
        tokenizer_sha256: &report.scores.tokenizer_sha256,
        assistant_prefix: &report.assistant_prefix,
        candidates: &report.candidates,
        baseline_log_odds: baseline,
        baseline_outside_pair_probability: report.scores.conditions[0].outside_pair_probability(),
        effects,
    })
}

fn same_score(actual: f64, reported: f64) -> bool {
    actual.is_finite() && (actual - reported).abs() <= 1e-10
}
