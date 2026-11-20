//! Compare complete replays of the same decision under different replacement texts.

use super::findings::{self, Observation};
use nexusweave::{replay, Run};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Condition<'a> {
    report: &'a replay::Replay,
    variants: Vec<&'a replay::Variant>,
    matches: Vec<usize>,
}

fn excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

pub(super) fn consistent_answer(trials: &[replay::Trial]) -> Option<String> {
    match trials.first() {
        Some(first @ replay::Trial::Completed { choice })
            if choice.kind == "model_output" && trials.iter().all(|trial| trial == first) =>
        {
            choice.outcome.as_str().map(excerpt)
        }
        _ => None,
    }
}

pub(super) fn arm(summary: &replay::Summary, trials: &[replay::Trial]) -> Value {
    json!({"matched":summary.matched,"attempted":summary.attempted,"answer":consistent_answer(trials)})
}

impl<'a> Condition<'a> {
    fn new(report: &'a replay::Replay) -> Option<Self> {
        let mut variants: Vec<_> = report.variants.iter().collect();
        variants.sort_by_key(|row| &row.intervention.context_id);
        if variants.is_empty()
            || variants.iter().any(|row| {
                row.difference_pp.is_none()
                    || !matches!(
                        row.intervention.operation,
                        replay::Operation::ReplaceText | replay::Operation::ReplaceTextFragment
                    )
            })
            || report
                .pair
                .as_ref()
                .is_some_and(|pair| pair.difference_pp.is_none())
        {
            return None;
        }
        let matches = variants
            .iter()
            .map(|row| row.summary.matched)
            .chain(report.pair.iter().map(|row| row.summary.matched))
            .collect();
        Some(Self {
            report,
            variants,
            matches,
        })
    }

    fn design(&self) -> String {
        let r = self.report;
        let anchors: Vec<_> = self
            .variants
            .iter()
            .map(|v| {
                let edit = &v.intervention;
                (
                    &edit.context_id,
                    &edit.pointer,
                    edit.operation,
                    &edit.text_range,
                    &v.captured_context_sha256,
                    v.summary.attempted,
                )
            })
            .collect();
        json!([
            r.location,
            r.runner_id,
            r.baseline_request_sha256,
            r.recorded,
            r.match_rule,
            r.sampling_design,
            r.baseline_summary,
            anchors,
            r.pair.as_ref().map(|p| p.summary.attempted),
            r.control_comparison.as_ref().map(|c| &c.control_context_id)
        ])
        .to_string()
    }

    fn evidence(&self) -> Value {
        let r = self.report;
        let edits: Vec<_> = self
            .variants
            .iter()
            .map(|v| {
                let mut evidence = arm(&v.summary, &v.trials);
                evidence["id"] = json!(v.intervention.context_id);
                evidence["replacement"] = json!(excerpt(&v.intervention.replacement));
                evidence
            })
            .collect();
        json!({"report_id":r.id,"baseline":arm(&r.baseline_summary,&r.baseline),"edits":edits,
            "joint":r.pair.as_ref().map(|p| arm(&p.summary,&p.trials))})
    }
}

pub(super) fn compare(run: &Run, reports: &[replay::Replay], findings: &mut Vec<Observation>) {
    if reports.len() < 2 {
        return;
    }
    // Order by exact replacement text so the selected example is stable across deliveries.
    let mut designs = BTreeMap::<String, BTreeMap<Vec<&str>, Vec<Condition>>>::new();
    for condition in reports.iter().filter_map(Condition::new) {
        let wording = condition
            .variants
            .iter()
            .map(|v| v.intervention.replacement.as_str())
            .collect();
        designs
            .entry(condition.design())
            .or_default()
            .entry(wording)
            .or_default()
            .push(condition);
    }
    for (design, conditions) in designs {
        let total = conditions.len();
        if total < 2 {
            continue;
        }
        let examples: Vec<_> = conditions
            .values()
            .filter_map(|runs| {
                let first = runs.iter().min_by_key(|c| &c.report.id)?;
                runs.iter()
                    .all(|c| c.matches == first.matches)
                    .then_some(first)
            })
            .collect();
        let Some(left) = examples.first() else {
            continue;
        };
        let Some(right) = examples.iter().find(|c| c.matches != left.matches) else {
            continue;
        };
        let r = left.report;
        let Some(call) = run.calls.iter().find(|c| c.id == r.location.call_id) else {
            continue;
        };
        let sources: Option<Vec<_>> = left
            .variants
            .iter()
            .map(|v| {
                call.context
                    .iter()
                    .find(|s| s.id == v.intervention.context_id)
                    .map(findings::source)
            })
            .collect();
        let Some(sources) = sources else { continue };
        findings.push(findings::occurrence("replacement", json!(["replacement", design]), call,
            json!({"sources":sources, "conditions":[left.evidence(),right.evidence()],
                "wordings":total, "consistent_wordings":examples.len(), "outcome":r.recorded.label, "model":call.model, "runner":r.runner_id,
                "decision_id":r.location.decision_id, "match_rule":r.match_rule})));
    }
}
