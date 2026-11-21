//! Compare every tested request edit against the same declared proposal.
use super::outcomes;
use a2context::{replay, replay::shadow, replay::TargetStatus, Call};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
struct Arm<'a> {
    label: &'a str,
    status: TargetStatus,
    counts: &'a replay::Summary,
    proposal: Option<Value>,
    changed: Option<bool>,
    control: bool,
    edit: Option<Value>,
}

fn arm<'a>(
    label: &'a str,
    counts: &'a replay::Summary,
    trials: &[replay::Trial],
    original: Option<&replay::Choice>,
) -> Arm<'a> {
    let actual = outcomes::consistent(trials);
    Arm {
        label,
        status: counts.target_status(),
        counts,
        proposal: actual.map(outcomes::preview),
        changed: original.zip(actual).map(|(a, b)| a != b),
        control: false,
        edit: None,
    }
}

fn recommendation(
    baseline: &Arm,
    edits: &[Arm],
    joint: Option<&Arm>,
    extended_target: bool,
) -> String {
    let all = || std::iter::once(baseline).chain(edits).chain(joint);
    if all().any(|a| a.status == TargetStatus::Incomplete) {
        return "Complete the missing or unscored trials before choosing an edit".into();
    }
    if all().any(|a| a.status == TargetStatus::Mixed) {
        return "Repeat the conditions with mixed results".into();
    }
    let control = edits.iter().find(|arm| arm.control);
    if control.is_some_and(|arm| arm.status != baseline.status) {
        return "Check why the negative control changed the target result".into();
    }
    if baseline.status == TargetStatus::Met {
        return if edits
            .iter()
            .chain(joint)
            .any(|a| a.status == TargetStatus::Missed)
        {
            "Keep the original and review the edits that lost the target"
        } else {
            "Keep the original for this request"
        }
        .into();
    }
    let mut met = edits
        .iter()
        .filter(|a| !a.control && a.status == TargetStatus::Met);
    match (met.next(), met.next(), joint.map(|a| a.status)) {
        (None, _, Some(TargetStatus::Met)) if control.is_some() => {
            "Repeat the edit with a different negative control".into()
        }
        (None, _, Some(TargetStatus::Met)) => "Test the combined edit on another request".into(),
        (None, _, _) if extended_target => "Inspect the text after the target answer".into(),
        (None, _, _) => "Try a different edit for this target".into(),
        (Some(_), _, Some(TargetStatus::Missed)) => {
            "Retest the successful single edits separately".into()
        }
        (Some(a), None, _) => format!("Retest the edit to {} on another request", a.label),
        _ => "Compare the successful single edits on another request".into(),
    }
}

pub(super) fn has_extended_target(report: &shadow::Report) -> bool {
    if report.expected.kind != "model_output"
        || !matches!(report.match_rule, replay::MatchRule::Exact)
    {
        return false;
    }
    let Some(target) = report.expected.outcome.as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    std::iter::once(report.baseline.as_slice())
        .chain(report.effects.iter().map(|a| a.result.trials.as_slice()))
        .chain(report.pair.iter().map(|p| p.result.trials.as_slice()))
        .filter_map(outcomes::consistent)
        .any(|choice| {
            choice.kind == report.expected.kind
                && choice.label == report.expected.label
                && choice
                    .outcome
                    .as_str()
                    .and_then(|s| s.strip_prefix(target))
                    .is_some_and(|suffix| !suffix.is_empty())
        })
}

pub(super) fn checks(call: &Call, probes: &[shadow::RecordedProbe]) -> Vec<Value> {
    let reports = call
        .shadow_probes
        .as_ref()
        .and_then(|c| c.reports.as_deref())
        .unwrap_or_default();
    if reports.is_empty() {
        return Vec::new();
    }
    let candidates = replay::candidates::recorded("target", call);
    let mut checks = Vec::new();
    for (probe, report) in probes.iter().zip(reports) {
        let shadow::Verification::Matched { measurement } = &probe.verification else {
            continue;
        };
        let original = outcomes::consistent(&report.baseline);
        let baseline = arm("Original", measurement.baseline, &report.baseline, original);
        let mut edits = Vec::with_capacity(report.effects.len());
        for (effect, raw) in measurement.effects.iter().zip(&report.effects) {
            let name = effect.source_id.unwrap_or(effect.context_id);
            let mut tested = arm(name, effect.summary, &raw.result.trials, original);
            let edit = &raw.result.intervention;
            tested.control = report
                .control_comparison
                .as_ref()
                .is_some_and(|c| c.control_context_id == edit.context_id);
            let mut chars = edit.replacement.chars();
            let mut replacement: String = chars.by_ref().take(100).collect();
            if chars.next().is_some() {
                replacement.push('…');
            }
            tested.edit = Some(
                json!({"context_id":effect.context_id, "pointer":edit.pointer,
                "operation":edit.operation, "replacement":replacement,"version":effect.source_version}),
            );
            edits.push(tested);
        }
        let joint = report.pair.as_ref().map(|pair| {
            arm(
                "Both edits",
                &pair.result.summary,
                &pair.result.trials,
                original,
            )
        });
        let next = recommendation(
            &baseline,
            &edits,
            joint.as_ref(),
            has_extended_target(report),
        );
        let token_paths: Vec<_> = candidates
            .iter()
            .filter_map(|probe| match &probe.verification {
                replay::candidates::Verification::Matched {
                    measurement: candidate,
                } => replay::candidates::target_path(candidate, measurement),
                _ => None,
            })
            .collect();
        checks.push(json!({ "token_paths":token_paths,
            "report_id": report.id, "runner": report.runner_id, "expected": outcomes::preview(measurement.expected),
            "match_rule": measurement.match_rule, "next": next, "baseline": baseline, "edits": edits, "joint": joint,
        }));
    }
    checks
}
