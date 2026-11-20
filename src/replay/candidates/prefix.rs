//! Compare a pair of continuations after a supplied assistant prefix with free generations.
use super::{margin, Candidate, Measurement, RecordedProbe, Verification};
use crate::{
    preflight,
    replay::{shadow, Choice, MatchRule, Trial},
};
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
            if reverses(baseline_margin, margin) {
                reversals.push(index);
            }
            conditions.push(json!({
                "label": name, "source":edit.and_then(|e|e.source_id),
                "original":edit.and_then(|e| e.context.text.as_deref()).map(excerpt), "edit": index.checked_sub(1).map(|i| &report.effects[i].intervention),
                "original_sha256":edit.map(|e| &e.captured_context_sha256),
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
                trial_run(measurement, trials, &shadow.evidence.field)
            })
            .collect();
        let verdict = verdict(&conditions, &reversals, &runs);
        reviews.push(json!({"report":report.id, "prefix":measurement.assistant_prefix,
            "verdict":verdict, "scorer":measurement.scorer_id, "tokenizer_sha256":measurement.tokenizer_sha256,
            "labels":measurement.candidates.each_ref().map(|c| &c.id),
            "alternatives":measurement.candidates.each_ref().map(|c| &c.continuation), "model":measurement.model_id,
            "revision":measurement.model_revision, "runtime":measurement.runtime, "conditions":conditions,
            "reversals":reversals, "runs":runs}));
    }
    reviews
}

fn reverses(original: f64, edited: f64) -> bool {
    (original > 0.0 && edited < 0.0) || (original < 0.0 && edited > 0.0)
}

/// Prepare one free generation per arm for the only edit that reversed preference.
/// Verifies the supplied scores and all source bindings before choosing the edit.
/// The shadow criterion records the original predicted literal. Directional
/// agreement is checked separately by `prefix_reviews` using both generated texts.
pub fn prefix_trial_case(
    request: &preflight::Request,
    candidate: &super::Report,
    runner_id: &str,
) -> Result<shadow::Case, String> {
    if runner_id.trim().is_empty() || candidate.assistant_prefix.is_empty() {
        return Err(
            "A prefix trial requires a named runner and a supplied assistant prefix.".into(),
        );
    }
    let (call, body, _) = preflight::pending_call(request.clone())?;
    let body = body.ok_or("A prefix trial requires the captured request body.")?;
    let original = call
        .recorded_request
        .as_ref()
        .ok_or("Missing recorded request.")?;
    let hash = super::sha256(original.body.as_bytes());
    let checked = super::recorded::verify(&call, &body, &hash, candidate)?;
    if call.model.as_deref() != Some(checked.model_id) {
        return Err(
            "The pending model ID must match the candidate scorer model for a prefix trial.".into(),
        );
    }
    let mut switches = checked
        .effects
        .iter()
        .zip(&candidate.effects)
        .filter(|(bound, _)| reverses(checked.baseline_log_odds, bound.log_odds));
    let (bound, raw) = switches
        .next()
        .filter(|_| switches.next().is_none())
        .ok_or("The scores must identify exactly one edit that reverses preference.")?;
    let mut intervention = raw.intervention.clone();
    intervention.context_id = bound.context_id.to_owned();
    let predicted = &checked.candidates[usize::from(checked.baseline_log_odds < 0.0)].continuation;
    let beginning = format!("{}{}", checked.assistant_prefix, predicted);
    Ok(shadow::Case {
        version: 1,
        id: format!("prefix-trial-{}", candidate.id),
        runner_id: runner_id.into(),
        expected: Choice {
            kind: "model_output".into(),
            label: "Model answer".into(),
            outcome: json!(beginning),
        },
        match_rule: MatchRule::TextCapture {
            pattern: format!(r"\A({})", regex::escape(&beginning)),
        },
        samples: 1,
        sampling_design: Default::default(),
        interventions: vec![intervention],
        test_pair: false,
        negative_control_context_id: None,
    })
}

// A direction is resolved only if every attempted trial in the arm began with
// the supplied prefix followed by the same unambiguous literal continuation.
fn unanimous_start(row: &Value) -> Option<usize> {
    let attempted = row["attempted"].as_u64().filter(|n| *n > 0)?;
    row["starts"]
        .as_array()?
        .iter()
        .position(|n| n.as_u64() == Some(attempted))
}

fn verdict(conditions: &[Value], reversals: &[usize], runs: &[Value]) -> &'static str {
    let [selected] = reversals else {
        let switched = runs.iter().any(|run| {
            let Some(rows) = run["rows"].as_array() else {
                return false;
            };
            let Some(original) = rows.first().and_then(unanimous_start) else {
                return false;
            };
            rows.iter()
                .skip(1)
                .filter_map(unanimous_start)
                .any(|s| s != original)
        });
        return if switched {
            "unselected_change"
        } else {
            "no_selection"
        };
    };
    let mut result = None;
    for run in runs {
        let rows = &run["rows"];
        if rows[*selected].is_null() {
            continue;
        }
        let status = match unanimous_start(&rows[0]).zip(unanimous_start(&rows[*selected])) {
            Some((a, b))
                if conditions[0]["preferred"] == a && conditions[*selected]["preferred"] == b =>
            {
                "confirmed"
            }
            Some(_) => "disagreed",
            None => "unresolved",
        };
        if result.is_some_and(|old| old != status) {
            return "mixed";
        }
        result = Some(status);
    }
    result.unwrap_or("untested")
}

fn trial_run(candidate: &Measurement, trials: &shadow::Measurement, origin: &str) -> Option<Value> {
    let report = trials.report;
    if trials.model != Some(candidate.model_id)
        || candidate.report.captured_request_sha256 != report.captured_request_sha256
    {
        return None;
    }
    let summarize =
        |rows: &[Trial]| count_starts(rows, candidate.assistant_prefix, candidate.candidates);
    let mut rows = vec![Some(summarize(&report.baseline))];
    for (edit, source) in candidate.report.effects.iter().zip(&candidate.effects) {
        let matched = report
            .effects
            .iter()
            .zip(&trials.effects)
            .find(|(e, bound)| {
                let actual = &e.result.intervention;
                e.request_sha256 == edit.request_sha256
                    && e.source_id == edit.source_id
                    && e.source_version == edit.source_version
                    && bound.context_id == source.context_id
                    && actual.pointer == edit.intervention.pointer
                    && actual.text_range == edit.intervention.text_range
                    && actual.operation == edit.intervention.operation
                    && actual.replacement == edit.intervention.replacement
            });
        rows.push(matched.map(|(e, _)| summarize(&e.result.trials)));
    }
    rows[1..]
        .iter()
        .any(Option::is_some)
        .then(|| json!({"origin":origin,"report":report.id,"runner":report.runner_id,"sampling":report.sampling_design,"rows":rows}))
}

fn excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let mut first: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        first.push('…');
    }
    first
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
            choice.outcome.as_str().map(excerpt)
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
    fn distinguishes_untested_incomplete_and_contradictory_trials() {
        let conditions = [json!({"preferred":0}), json!({"preferred":1})];
        let trials = |starts| json!({"attempted":2,"starts":starts});
        let run = |edited| json!({"rows":[trials([2,0]),edited]});
        let confirmed = run(trials([0, 2]));
        let held = run(trials([2, 0]));
        assert_eq!(verdict(&conditions, &[1], &[run(Value::Null)]), "untested");
        assert_eq!(
            verdict(&conditions, &[1], &[run(trials([0, 1]))]),
            "unresolved"
        );
        assert_eq!(
            verdict(&conditions, &[1], &[run(trials([1, 1]))]),
            "unresolved"
        );
        assert_eq!(
            verdict(&conditions, &[1], std::slice::from_ref(&held)),
            "disagreed"
        );
        assert_eq!(
            verdict(&conditions, &[1], std::slice::from_ref(&confirmed)),
            "confirmed"
        );
        assert_eq!(
            verdict(&conditions, &[], std::slice::from_ref(&confirmed)),
            "unselected_change"
        );
        assert_eq!(verdict(&conditions, &[], &[]), "no_selection");
        assert_eq!(verdict(&conditions, &[1], &[confirmed, held]), "mixed");
    }

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
