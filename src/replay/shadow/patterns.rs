//! Compare recurring effects on declared proposals across recorded scopes.

use super::{recorded, Measurement, RecordedEffect, Verification};
use crate::{
    canonical_json,
    replay::{
        matching::{Matcher, Projection},
        probe_patterns::{self, Kind, Measure, Observation},
        sha256, Operation, SamplingDesign,
    },
    turn_evidence::excerpt,
    Run,
};
use serde::Serialize;
use serde_json::{json, Value};

pub use crate::replay::probe_patterns::Request;

#[derive(Serialize)]
struct Setup<'a> {
    source_id: &'a str,
    source_version: Option<&'a str>,
    runner_id: &'a str,
    sampling_design: SamplingDesign,
    operation: Operation,
    expected_kind: &'a str,
    expected_label: Option<&'a str>,
    match_rule: &'static str,
    target_excerpt: Option<Value>,
}

/// Query verified shadow effects without pooling trials. `include_target`
/// includes the declared proposal label and an excerpt of the scored value.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    include_target: bool,
) -> Result<Value, String> {
    probe_patterns::query(request, runs, Kind::Shadow, |run, call| {
        let mut rows = Vec::new();
        for probe in recorded(&run.id, call) {
            let Verification::Matched { measurement } = probe.verification else {
                continue;
            };
            let (target_id, rule, projection) = target(&measurement)?;
            let target_excerpt =
                (include_target && rule != "kind_and_label").then(|| match projection {
                    Value::String(text) => excerpt(&text),
                    value => excerpt(&canonical_json(&value)),
                });
            for effect in &measurement.effects {
                let Some(source_id) = effect.source_id.filter(|id| request.includes_source(id))
                else {
                    continue;
                };
                let key = group_key(&measurement, effect, source_id, &target_id);
                rows.push(Observation {
                    key,
                    setup: Setup {
                        source_id,
                        source_version: effect.source_version,
                        runner_id: measurement.runner_id,
                        sampling_design: measurement.sampling_design,
                        operation: effect.intervention.operation,
                        expected_kind: &measurement.expected.kind,
                        expected_label: include_target.then_some(&measurement.expected.label),
                        match_rule: rule,
                        target_excerpt: target_excerpt.clone(),
                    },
                    report_id: probe.report_id.expect("matched probe ID"),
                    context_id: effect.context_id,
                    measure: Measure::Shadow {
                        baseline: measurement.baseline,
                        edited: effect.summary,
                        difference_pp: effect.difference_pp,
                        difference_95_interval_pp: effect.difference_95_interval_pp,
                    },
                });
            }
        }
        Ok(rows)
    })
}

fn target(measurement: &Measurement<'_>) -> Result<(String, &'static str, Value), String> {
    let rule = measurement.match_rule.normalized();
    let expected = measurement.expected;
    let projection = Matcher::new(&rule, expected)?
        .project(expected)
        .ok_or("The verified probe target cannot be projected.")?;
    let (kind, outcome) = match projection {
        Projection::Label => ("kind_and_label", Value::Null),
        Projection::Exact(value) => ("exact", json!(value)),
        Projection::Fields(values) => ("outcome_fields", json!(values)),
        Projection::Capture(value) => ("text_capture", json!(value)),
    };
    let key = canonical_json(&json!((&expected.kind, &expected.label, rule, &outcome)));
    Ok((sha256(key.as_bytes()), kind, outcome))
}

fn group_key(
    measurement: &Measurement<'_>,
    effect: &RecordedEffect<'_>,
    source_id: &str,
    target_id: &str,
) -> String {
    let identity = json!((
        "nexusweave.shadow_pattern.v1",
        source_id,
        effect.source_version,
        &effect.context.kind,
        &effect.context.role,
        effect.captured_context_sha256,
        effect.intervention.operation,
        &effect.intervention.replacement,
        measurement.runner_id,
        measurement.sampling_design,
        target_id,
    ));
    sha256(identity.to_string().as_bytes())
}
