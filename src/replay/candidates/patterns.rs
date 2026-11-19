//! Find recurring preference shifts under comparable candidate probes.

use super::{recorded, sha256, Measurement, RecordedEffect, Verification};
use crate::{
    replay::{
        probe_patterns::{self, Kind, Measure, Observation},
        Operation,
    },
    Run,
};
use serde::Serialize;
use serde_json::Value;

pub use crate::replay::probe_patterns::Request;

/// Query verified probe measurements within one or two trace scopes.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
) -> Result<Value, String> {
    probe_patterns::query(request, runs, Kind::Candidate, |run, call| {
        let mut rows = Vec::new();
        for probe in recorded(&run.id, call) {
            let Verification::Matched { measurement } = probe.verification else {
                continue;
            };
            for effect in &measurement.effects {
                let Some(source_id) = effect.source_id.filter(|id| request.includes_source(id))
                else {
                    continue;
                };
                let (key, setup) = setup(&measurement, effect, source_id)?;
                rows.push(Observation {
                    key,
                    setup,
                    report_id: probe.report_id.expect("matched probe ID"),
                    context_id: effect.context_id,
                    measure: Measure::Candidate {
                        baseline_log_odds: measurement.baseline_log_odds,
                        edited_log_odds: effect.log_odds,
                        change_log_odds: effect.change_log_odds,
                    },
                });
            }
        }
        Ok(rows)
    })
}

#[derive(Serialize)]
struct Setup<'a> {
    source_id: &'a str,
    source_version: Option<&'a str>,
    scorer_id: &'a str,
    model_id: &'a str,
    model_revision: &'a str,
    runtime: &'a str,
    candidate_ids: [&'a str; 2],
    operation: Operation,
}

fn setup<'a>(
    measurement: &Measurement<'a>,
    effect: &RecordedEffect<'a>,
    source_id: &'a str,
) -> Result<(String, Setup<'a>), String> {
    let setup = Setup {
        source_id,
        source_version: effect.source_version,
        scorer_id: measurement.scorer_id,
        model_id: measurement.model_id,
        model_revision: measurement.model_revision,
        runtime: measurement.runtime,
        candidate_ids: measurement
            .candidates
            .each_ref()
            .map(|candidate| candidate.id.as_str()),
        operation: effect.operation,
    };
    let identity = serde_json::to_vec(&(
        "nexusweave.candidate_pattern.v1",
        &setup,
        measurement.candidates,
        measurement.assistant_prefix,
        &measurement.scores.tokenizer_sha256,
        &measurement.scores.assistant_prefix_token_ids,
        &measurement.scores.candidate_token_ids,
        &effect.context.kind,
        &effect.context.role,
        &effect.captured_context_sha256,
        effect.replacement,
    ))
    .map_err(|error| error.to_string())?;
    Ok((sha256(&identity), setup))
}
