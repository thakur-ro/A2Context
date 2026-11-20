//! Compare a pair of continuations after a supplied assistant prefix with free generations.
use super::{margin, Candidate, Measurement, RecordedProbe, Verification};
use crate::replay::{shadow, Trial};
use serde_json::{json, Value};

/// Summarize verified, nonempty-prefix candidate reports. A source is selected only
/// when exactly one single edit strictly reverses preference. Proposal counts use
/// independent shadow trials on the same original and the exact same source edit.
pub fn prefix_reviews(
    candidates: &[RecordedProbe],
    shadows: &[shadow::RecordedProbe],
) -> Vec<Value> {
    let mut reviews = Vec::new();
    for probe in candidates {
        let Verification::Matched { measurement } = &probe.verification else {
            continue;
        };
        if measurement.assistant_prefix.is_empty() {
            continue;
        }
        let report = measurement.report;
        let baseline_margin = margin(&measurement.scores.conditions[0]);
        let mut reversals = Vec::new();
        let mut conditions = Vec::new();
        for (index, row) in measurement
            .scores
            .conditions
            .iter()
            .take(report.effects.len() + 1)
            .enumerate()
        {
            let edit = index.checked_sub(1).map(|i| &measurement.effects[i]);
            let name = edit.map_or("Original", |e| e.source_id.unwrap_or(e.context_id));
            let margin = margin(row);
            if (margin > 0.0 && baseline_margin < 0.0) || (margin < 0.0 && baseline_margin > 0.0) {
                reversals.push(index);
            }
            conditions.push(json!({
                "label": name, "edit": index.checked_sub(1).map(|i| &report.effects[i].intervention),
                "version": edit.and_then(|e| e.source_version), "change_log_odds":margin - baseline_margin,
                "probabilities": row.log_probs.map(f64::exp), "outside": row.outside_pair_probability(),
                "preferred": if margin > 0.0 { Some(0) } else if margin < 0.0 { Some(1) } else { None }
            }));
        }
        let runs: Vec<_> = shadows
            .iter()
            .filter(|shadow| shadow.run_id == probe.run_id && shadow.call_id == probe.call_id)
            .filter_map(|shadow| {
                let shadow::Verification::Matched {
                    measurement: trials,
                } = &shadow.verification
                else {
                    return None;
                };
                trial_run(measurement, trials)
            })
            .collect();
        reviews.push(json!({"report":report.id, "prefix":measurement.assistant_prefix,
            "labels":measurement.candidates.each_ref().map(|c| &c.id),
            "alternatives":measurement.candidates.each_ref().map(|c| &c.continuation), "model":measurement.model_id,
            "revision":measurement.model_revision, "runtime":measurement.runtime, "conditions":conditions,
            "reversals":reversals, "runs":runs}));
    }
    reviews
}

fn trial_run(candidate: &Measurement, trials: &shadow::Measurement) -> Option<Value> {
    let report = trials.report;
    if trials.model != Some(candidate.model_id)
        || candidate.report.captured_request_sha256 != report.captured_request_sha256
    {
        return None;
    }
    let summarize =
        |rows: &[Trial]| count_starts(rows, candidate.assistant_prefix, candidate.candidates);
    let mut rows = vec![Some(summarize(&report.baseline))];
    for edit in &candidate.report.effects {
        let matched = report.effects.iter().find(|e| {
            e.request_sha256 == edit.request_sha256
                && e.source_id == edit.source_id
                && e.source_version == edit.source_version
                && e.result.intervention == edit.intervention
        });
        rows.push(matched.map(|e| summarize(&e.result.trials)));
    }
    rows[1..]
        .iter()
        .any(Option::is_some)
        .then(|| json!({"report":report.id,"runner":report.runner_id,"rows":rows}))
}

// The token sequences are disjoint, but decoded strings can overlap. Only count a
// named beginning when exactly one literal continuation matches the generated text.
fn count_starts(trials: &[Trial], prefix: &str, alternatives: &[Candidate; 2]) -> Value {
    let (mut text, mut followed, mut other_outputs, mut failed) = (0, 0, 0, 0);
    let mut starts = [0, 0];
    for trial in trials {
        let Trial::Completed { choice } = trial else {
            failed += 1;
            continue;
        };
        let Some(answer) = choice
            .outcome
            .as_str()
            .filter(|_| choice.kind == "model_output")
        else {
            other_outputs += 1;
            continue;
        };
        text += 1;
        if let Some(rest) = answer.strip_prefix(prefix) {
            followed += 1;
            match alternatives
                .each_ref()
                .map(|a| rest.starts_with(&a.continuation))
            {
                [true, false] => starts[0] += 1,
                [false, true] => starts[1] += 1,
                _ => {}
            }
        }
    }
    let example = match trials {
        [Trial::Completed { choice }] if choice.kind == "model_output" => {
            choice.outcome.as_str().map(|s| {
                let mut chars = s.chars();
                let mut preview: String = chars.by_ref().take(160).collect();
                if chars.next().is_some() {
                    preview.push('…');
                }
                preview
            })
        }
        _ => None,
    };
    json!({"example":example,"attempted":trials.len(),"text":text,"followed":followed,"starts":starts,
        "other_outputs":other_outputs,"failed":failed})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::Choice;

    #[test]
    fn separates_literal_starts_from_other_text_tool_calls_and_failed_trials() {
        let choices = ["3", "30"].map(|text| Candidate {
            id: text.into(),
            continuation: text.into(),
        });
        let output = |kind: &str, text: &str| Trial::Completed {
            choice: Choice {
                kind: kind.into(),
                label: "choice".into(),
                outcome: json!(text),
            },
        };
        let trials = [
            output("model_output", "Limit: 3 days"),
            output("model_output", "Limit: 30 days"),
            output("model_output", "Limit: \"30\""),
            output("model_output", "30"),
            output("tool_call", "Limit: 3"),
            Trial::Failed,
        ];
        let result = count_starts(&trials, "Limit: ", &choices);
        assert_eq!(
            result,
            json!({"example":null,"attempted":6,"text":4,"followed":3,"starts":[1,0],"other_outputs":1,"failed":1})
        );
    }
}
