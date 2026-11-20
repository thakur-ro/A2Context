use super::findings::{self, Observation};
use nexusweave::{activation, pending, replay, Call, Run};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct Action {
    title: &'static str,
    next: &'static str,
    basis: &'static str,
}

pub fn action(kind: &str) -> Action {
    let (title, next, basis) = match kind {
        "conflict" => (
            "Resolve the competing rules",
            "Confirm which source should govern this scope. Test changing only the other declaration against the answer or tool choice you expect.",
            "Observed input disagreement",
        ),
        "duplicate" => (
            "Test removing one copy",
            "Keep one copy in its original role and test removing the other. Compare the named outcome before trimming the input in production.",
            "Observed exact repetition",
        ),
        "candidate" => (
            "Test the source that shifted the choice",
            "Repeat the comparison at the same prefix and include an unrelated control edit. Check the full proposed answer or tool arguments before adopting the change.",
            "Measured preference at a fixed prefix",
        ),
        "shadow" => (
            "Verify the edited proposal",
            "Check the complete proposed answer or tool arguments on more representative inputs before adopting the edit.",
            "Measured proposal rate",
        ),
        "token" => (
            "Investigate the source at this output step",
            "Repeat this intervention with neighboring tokens and a control source. Test fresh continuations to see whether the change carries forward.",
            "Measured token scores on a fixed output path",
        ),
        _ => (
            "Check whether the edit generalizes",
            "Repeat this edit on other requests in the same scope. Keep the outcome rule and runner configuration comparable.",
            "Measured replay outcome",
        ),
    };
    Action { title, next, basis }
}

pub fn preview(input: pending::Request) -> Result<Value, String> {
    let (call, scope) = pending::recommendation_input(input)?;
    let run = Run {
        schema_version: nexusweave::SCHEMA_VERSION,
        id: "pending".into(),
        source: "pending".into(),
        organization_id: None,
        user_id: None,
        conversation_id: None,
        calls: vec![call],
        spans: vec![],
    };
    let call = &run.calls[0];
    let mut observations = findings::derive(&run, &[], &[])?;
    observations.retain(|item| {
        item.kind != "conflict"
            || scope.as_ref().is_none_or(|scope| {
                item.detail["rule"].as_str() == Some(&scope.key)
                    && item.detail["scope"].as_str() == Some(&scope.scope)
            })
    });
    observations.sort_by_key(|item| match item.kind {
        "candidate" | "shadow" => 0,
        "conflict" => 1,
        _ => 2,
    });
    let total = observations.len();
    let items = observations.into_iter().take(15).map(|item| json!({
        "kind": item.kind, "id": item.group, "detail": item.detail, "action": action(item.kind),
    })).collect::<Vec<_>>();
    let unverified = replay::candidates::recorded("pending", call)
        .into_iter()
        .filter_map(|probe| match probe.verification {
            replay::candidates::Verification::Unverified { issue } => Some(issue),
            _ => None,
        })
        .chain(
            replay::shadow::recorded("pending", call)
                .into_iter()
                .filter_map(|probe| match probe.verification {
                    replay::shadow::Verification::Unverified { issue } => Some(issue),
                    _ => None,
                }),
        )
        .take(10)
        .collect::<Vec<_>>();
    Ok(json!({
        "version": 1, "model": call.model, "captured_inputs": call.context.len(),
        "total": total, "recommendations": items, "unverified_probes": unverified,
    }))
}

pub fn probes(run: &Run, call: &Call, findings: &mut Vec<Observation>) {
    for probe in replay::candidates::recorded(&run.id, call) {
        let replay::candidates::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        for effect in measurement
            .effects
            .iter()
            .filter(|effect| effect.change_log_odds.abs() > 1e-10)
        {
            let Some(item) = call
                .context
                .iter()
                .find(|item| item.id == effect.context_id)
            else {
                continue;
            };
            let key = json!([
                "candidate",
                measurement.scorer_id,
                measurement.model_id,
                measurement.model_revision,
                measurement.runtime,
                measurement.tokenizer_sha256,
                measurement.assistant_prefix,
                measurement.candidates,
                findings::identity(run, item),
                effect.operation,
                effect.replacement
            ]);
            findings.push(findings::occurrence("candidate", key, call, json!({
                "report_id": probe.report_id, "sources": [findings::source(item)], "alternatives": measurement.candidates,
                "baseline": measurement.baseline_log_odds, "edited": effect.log_odds, "change": effect.change_log_odds,
                "model": measurement.model_id, "revision": measurement.model_revision, "runtime": measurement.runtime,
                "prefix": measurement.assistant_prefix, "replacement": effect.replacement, "operation": effect.operation,
                "other_edits": measurement.effects.iter().filter(|other| other.context_id != effect.context_id).take(4)
                    .map(|other| json!({"source": other.source_id, "id": other.context_id, "change": other.change_log_odds}))
                    .collect::<Vec<_>>(),
            })));
        }
    }
    for probe in replay::shadow::recorded(&run.id, call) {
        let replay::shadow::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        for effect in &measurement.effects {
            let Some(change) = effect.difference_pp.filter(|change| *change != 0.0) else {
                continue;
            };
            let Some(item) = call
                .context
                .iter()
                .find(|item| item.id == effect.context_id)
            else {
                continue;
            };
            let key = json!([
                "shadow",
                measurement.runner_id,
                call.model,
                measurement.expected,
                measurement.match_rule,
                measurement.sampling_design,
                findings::identity(run, item),
                effect.intervention.operation,
                effect.intervention.replacement
            ]);
            findings.push(findings::occurrence("shadow", key, call, json!({
                "report_id": probe.report_id, "sources": [findings::source(item)], "outcome": measurement.expected,
                "effect_pp": change, "model": call.model, "runner": measurement.runner_id,
                "measurement": {"baseline": measurement.baseline, "variant": effect.summary, "interval": effect.difference_95_interval_pp},
                "replacement": effect.intervention.replacement, "operation": effect.intervention.operation,
            })));
        }
    }
}

pub fn token_measurements(
    run: &Run,
    reports: &[activation::Report],
    findings: &mut Vec<Observation>,
) {
    for report in reports {
        let Some(landmark) = activation::output_scores::landmark(report) else {
            continue;
        };
        let Some(call) = run
            .calls
            .iter()
            .find(|call| call.id == report.location().call_id)
        else {
            continue;
        };
        let Some(item) = call
            .context
            .iter()
            .find(|item| item.id == landmark.context_id)
        else {
            continue;
        };
        findings.push(findings::occurrence("token", json!(["token", run.id, call.id, report.id()]), call, json!({
            "report_id": report.id(), "sources": [findings::source(item)], "location": report.location(),
            "landmark": landmark,
        })));
    }
}
