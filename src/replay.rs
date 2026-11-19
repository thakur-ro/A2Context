//! Controlled request replays. A runner proposes a decision and returns it as JSON.

pub mod candidates;
mod capture;
mod distribution;
mod matching;
pub mod shadow;
pub(crate) use capture::{
    capture_availability, capture_profiles, captured_container, has_input_profile,
};
pub use capture::{
    request_field_matches, CallCapture, CaptureAvailability, ContextFieldMatch, ItemCapture,
    RequestField, RequestFieldKind,
};
pub use distribution::{outcome_distributions, OutcomeDistribution};
pub(crate) use matching::valid_pointer;
pub use matching::MatchRule;
use matching::Matcher;

use crate::{Availability, ContextItem, Run, TextRange};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub run_id: String,
    pub call_id: String,
    pub decision_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Choice {
    pub kind: String,
    pub label: String,
    pub outcome: Value,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    pub version: u32,
    pub id: String,
    pub runner_id: String,
    pub location: Location,
    pub request: Value,
    pub samples: usize,
    pub interventions: Vec<Intervention>,
    #[serde(default)]
    pub test_pair: bool,
    #[serde(default)]
    pub negative_control_context_id: Option<String>,
    #[serde(default)]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub sampling_design: SamplingDesign,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SamplingDesign {
    #[default]
    Unspecified,
    Independent,
}

impl SamplingDesign {
    pub fn is_unspecified(&self) -> bool {
        *self == Self::Unspecified
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Intervention {
    pub context_id: String,
    pub pointer: String,
    #[serde(default, skip_serializing_if = "Operation::is_text")]
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_range: Option<TextRange>,
    pub replacement: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    #[default]
    ReplaceText,
    ReplaceTextFragment,
    ReplaceJson,
    RemoveJson,
}

impl Operation {
    pub fn is_text(&self) -> bool {
        *self == Self::ReplaceText
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Trial {
    Completed { choice: Choice },
    Failed,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Replay {
    pub version: u32,
    pub id: String,
    pub runner_id: String,
    pub location: Location,
    pub recorded: Choice,
    #[serde(default, skip_serializing_if = "MatchRule::is_exact")]
    pub match_rule: MatchRule,
    #[serde(default, skip_serializing_if = "SamplingDesign::is_unspecified")]
    pub sampling_design: SamplingDesign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescored_from: Option<String>,
    pub baseline_request_sha256: String,
    pub baseline: Vec<Trial>,
    pub baseline_summary: Summary,
    pub variants: Vec<Variant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<Pair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_comparison: Option<ControlComparison>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Variant {
    pub intervention: Intervention,
    pub captured_context_sha256: String,
    pub trials: Vec<Trial>,
    pub summary: Summary,
    pub difference_pp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difference_95_interval_pp: Option<ReplayInterval>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Pair {
    pub trials: Vec<Trial>,
    pub summary: Summary,
    pub difference_pp: Option<f64>,
    pub interaction_pp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difference_95_interval_pp: Option<ReplayInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_95_interval_pp: Option<ReplayInterval>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ControlComparison {
    pub control_context_id: String,
    pub target_minus_control_pp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_minus_control_95_interval_pp: Option<ReplayInterval>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReplayInterval {
    pub lower: i16,
    pub upper: i16,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub attempted: usize,
    pub completed: usize,
    pub matched: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unscored: usize,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

impl Summary {
    fn all_scored(&self) -> bool {
        self.completed == self.attempted && self.unscored == 0
    }
}

struct PreparedCase<'a> {
    recorded: Choice,
    baseline_request_sha256: String,
    match_rule: MatchRule,
    edits: Vec<PreparedEdit<'a>>,
    control: Option<usize>,
}

fn prepare_case<'case, 'run>(
    case: &'case Case,
    runs: impl IntoIterator<Item = &'run Run>,
) -> Result<PreparedCase<'case>, String> {
    if case.version != 1 || case.id.trim().is_empty() || case.runner_id.trim().is_empty() {
        return Err("expected a named replay case and runner with version 1".into());
    }
    if case.samples == 0 || case.interventions.is_empty() || !case.request.is_object() {
        return Err(
            "replay needs a request object, at least one sample, and an intervention".into(),
        );
    }
    if case.test_pair && case.interventions.len() != 2 {
        return Err("a pair test requires exactly two interventions".into());
    }
    let (call, recorded) = locate(runs, &case.location)?;
    let baseline_request_sha256 = sha256(case.request.to_string().as_bytes());
    verified_request(call, &baseline_request_sha256)?;
    let edits = prepare_edits(call, &case.request, &case.interventions, case.test_pair)?;
    let control = case
        .negative_control_context_id
        .as_deref()
        .map(|id| control_index(id, &case.interventions))
        .transpose()?;
    Ok(PreparedCase {
        recorded,
        baseline_request_sha256,
        match_rule: case.match_rule.normalized(),
        edits,
        control,
    })
}

fn prepare_edits<'a>(
    call: &crate::Call,
    request: &Value,
    interventions: &'a [Intervention],
    test_pair: bool,
) -> Result<Vec<PreparedEdit<'a>>, String> {
    let mut seen = BTreeSet::new();
    let mut edits = Vec::new();
    for intervention in interventions {
        let edit = prepare_edit(call, intervention)?;
        if !seen.insert(&intervention.context_id) {
            return Err(format!(
                "duplicate intervention for {}",
                intervention.context_id
            ));
        }
        edit.check(request)?;
        edits.push(edit);
    }
    if test_pair {
        let [first, second] = interventions else {
            return Err("a pair test requires exactly two interventions".into());
        };
        validate_pair(first, second)?;
    }
    Ok(edits)
}

pub fn validate_case<'a>(
    case: &Case,
    runs: impl IntoIterator<Item = &'a Run>,
) -> Result<(), String> {
    let prepared = prepare_case(case, runs)?;
    Matcher::new(&prepared.match_rule, &prepared.recorded)?;
    Ok(())
}

pub fn execute(
    case: &Case,
    runs: &[Run],
    sample: impl FnMut(&Value) -> Option<Choice>,
) -> Result<Replay, String> {
    let PreparedCase {
        recorded,
        baseline_request_sha256,
        match_rule,
        edits,
        control,
    } = prepare_case(case, runs)?;
    let matcher = Matcher::new(&match_rule, &recorded)?;
    let requests = edited_requests(case.request.clone(), &edits, case.test_pair)?;
    let mut trials = sample_conditions(&requests, case.samples, sample);
    let paired_trials = case
        .test_pair
        .then(|| trials.pop().expect("pair condition"));
    let mut trials = trials.into_iter();
    let baseline = trials.next().expect("baseline condition");
    let baseline_summary = summarize(&baseline, &matcher);
    let variants: Vec<_> = case
        .interventions
        .iter()
        .cloned()
        .zip(edits)
        .zip(trials)
        .map(|((mut intervention, edit), trials)| {
            if intervention.operation == Operation::ReplaceJson {
                intervention.replacement =
                    edit.after.as_ref().expect("JSON replacement").to_string();
            }
            let summary = summarize(&trials, &matcher);
            let difference_pp = difference(&baseline_summary, &summary);
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
    let pair = paired_trials.map(|trials| {
        let summary = summarize(&trials, &matcher);
        let difference_pp = difference(&baseline_summary, &summary);
        let interaction_pp = interaction(&baseline_summary, &variants, &summary);
        Pair {
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
            trials,
            summary,
        }
    });
    let control_comparison =
        control.map(|index| compare_control(index, &variants, case.sampling_design));
    Ok(Replay {
        version: 1,
        id: case.id.clone(),
        runner_id: case.runner_id.clone(),
        location: case.location.clone(),
        recorded,
        match_rule,
        sampling_design: case.sampling_design,
        rescored_from: None,
        baseline_request_sha256,
        baseline,
        baseline_summary,
        variants,
        pair,
        control_comparison,
    })
}

fn edited_requests(
    request: Value,
    edits: &[PreparedEdit<'_>],
    test_pair: bool,
) -> Result<Vec<Value>, String> {
    let mut requests = vec![request];
    for edit in edits {
        let mut variant = requests[0].clone();
        edit.apply(&mut variant)?;
        requests.push(variant);
    }
    if test_pair {
        let mut both = requests[0].clone();
        let mut pair = [&edits[0], &edits[1]];
        // Apply text edits from right to left and replace before removing original array entries.
        pair.sort_by_key(|edit| {
            let (parent, key) = edit.pointer.rsplit_once('/').expect("validated pointer");
            (
                edit.after.is_none(),
                Reverse(parent.matches('/').count()),
                Reverse(key.parse::<usize>().unwrap_or(0)),
                Reverse(edit.text_range.map(|range| range.start_byte)),
            )
        });
        for edit in pair {
            edit.apply(&mut both)?;
        }
        requests.push(both);
    }
    Ok(requests)
}

fn sample_conditions(
    requests: &[Value],
    samples: usize,
    mut sample: impl FnMut(&Value) -> Option<Choice>,
) -> Vec<Vec<Trial>> {
    let mut trials = vec![Vec::with_capacity(samples); requests.len()];
    // Rotate condition order across rounds so baseline is not always sampled first.
    for round in 0..samples {
        for offset in 0..requests.len() {
            let index = (round + offset) % requests.len();
            trials[index].push(match sample(&requests[index]) {
                Some(choice) => Trial::Completed { choice },
                None => Trial::Failed,
            });
        }
    }
    trials
}

pub fn rescore(
    mut report: Replay,
    runs: &[Run],
    new_id: &str,
    rule: MatchRule,
) -> Result<Replay, String> {
    validate_report(&report, runs)?;
    if new_id.trim().is_empty()
        || new_id == report.id
        || report.rescored_from.as_deref() == Some(new_id)
    {
        return Err("rescoring requires a new replay ID".into());
    }
    let rule = rule.normalized();
    let matcher = Matcher::new(&rule, &report.recorded)?;
    report.baseline_summary = summarize(&report.baseline, &matcher);
    for variant in &mut report.variants {
        variant.summary = summarize(&variant.trials, &matcher);
        variant.difference_pp = difference(&report.baseline_summary, &variant.summary);
        variant.difference_95_interval_pp = None;
    }
    if let Some(pair) = &mut report.pair {
        pair.summary = summarize(&pair.trials, &matcher);
        pair.difference_pp = difference(&report.baseline_summary, &pair.summary);
        pair.interaction_pp =
            interaction(&report.baseline_summary, &report.variants, &pair.summary);
        pair.difference_95_interval_pp = None;
        pair.interaction_95_interval_pp = None;
    }
    if let Some(control) = &mut report.control_comparison {
        let index = control_index(
            &control.control_context_id,
            report.variants.iter().map(|variant| &variant.intervention),
        )?;
        *control = compare_control(index, &report.variants, SamplingDesign::Unspecified);
    }
    let prior_id = std::mem::replace(&mut report.id, new_id.into());
    if report.rescored_from.is_none() {
        report.rescored_from = Some(prior_id);
    }
    report.match_rule = rule;
    Ok(report)
}

pub fn validate_report(report: &Replay, runs: &[Run]) -> Result<(), String> {
    let (call, recorded) = locate(runs, &report.location)?;
    if report.version != 1
        || report.id.trim().is_empty()
        || report
            .rescored_from
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id == &report.id)
        || report.runner_id.trim().is_empty()
        || report.recorded != recorded
        || report.baseline.is_empty()
        || report.variants.is_empty()
        || report.baseline_request_sha256.len() != 64
        || !report
            .baseline_request_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("replay does not match its recorded decision or has no trials".into());
    }
    let original = verified_request(call, &report.baseline_request_sha256)?;
    let matcher = Matcher::new(&report.match_rule, &recorded)?;
    if report.baseline_summary != summarize(&report.baseline, &matcher) {
        return Err("replay baseline scores do not match trial outcomes".into());
    }
    let samples = report.baseline.len();
    let sampling_design = if report.rescored_from.is_some() {
        SamplingDesign::Unspecified
    } else {
        report.sampling_design
    };
    let mut seen = BTreeSet::new();
    for variant in &report.variants {
        let edit = prepare_edit(call, &variant.intervention)?;
        if edit.captured_context_sha256 != variant.captured_context_sha256 {
            return Err("replay was recorded against different context text or JSON".into());
        }
        if let Some(request) = &original {
            edit.check(request)?;
        }
        if variant.trials.len() != report.baseline.len()
            || !seen.insert(&variant.intervention.context_id)
        {
            return Err(
                "replay contains unbalanced trials or duplicate intervention targets".into(),
            );
        }
        if variant.summary != summarize(&variant.trials, &matcher)
            || variant.difference_pp != difference(&report.baseline_summary, &variant.summary)
        {
            return Err("replay scores do not match their trial outcomes".into());
        }
        if variant.difference_95_interval_pp
            != interval(sampling_design, variant.difference_pp, samples, 2)
        {
            return Err("replay interval does not match its sampling design and scores".into());
        }
    }
    if let Some(pair) = &report.pair {
        let [first, second] = &report.variants[..] else {
            return Err("a pair report requires exactly two single interventions".into());
        };
        validate_pair(&first.intervention, &second.intervention)?;
        if pair.trials.len() != report.baseline.len()
            || pair.summary != summarize(&pair.trials, &matcher)
            || pair.difference_pp != difference(&report.baseline_summary, &pair.summary)
            || pair.interaction_pp
                != interaction(&report.baseline_summary, &report.variants, &pair.summary)
        {
            return Err("pair replay scores do not match their trial outcomes".into());
        }
        if pair.difference_95_interval_pp
            != interval(sampling_design, pair.difference_pp, samples, 2)
            || pair.interaction_95_interval_pp
                != interval(sampling_design, pair.interaction_pp, samples, 4)
        {
            return Err(
                "pair replay intervals do not match their sampling design and scores".into(),
            );
        }
    }
    if let Some(control) = &report.control_comparison {
        let index = control_index(
            &control.control_context_id,
            report.variants.iter().map(|variant| &variant.intervention),
        )?;
        if *control != compare_control(index, &report.variants, sampling_design) {
            return Err(
                "negative control comparison does not match the trial outcomes and sampling design"
                    .into(),
            );
        }
    }
    Ok(())
}

fn verified_request(call: &crate::Call, baseline_sha256: &str) -> Result<Option<Value>, String> {
    let Some(recorded) = &call.recorded_request else {
        return Ok(None);
    };
    let request = recorded.parse()?;
    if baseline_sha256 != sha256(request.to_string().as_bytes()) {
        return Err(
            "replay baseline differs from the inference request captured on this call".into(),
        );
    }
    Ok(Some(request))
}

struct PreparedEdit<'a> {
    pointer: &'a str,
    text_range: Option<TextRange>,
    before: Value,
    after: Option<Value>,
    captured_context_sha256: String,
}

impl PreparedEdit<'_> {
    fn check(&self, request: &Value) -> Result<(), String> {
        let slot = request
            .pointer(self.pointer)
            .ok_or_else(|| format!("request pointer not found: {}", self.pointer))?;
        let matches = if let Some(range) = self.text_range {
            slot.as_str()
                .and_then(|text| text.get(range.bytes()))
                .is_some_and(|fragment| self.before.as_str() == Some(fragment))
        } else {
            *slot == self.before
        };
        if !matches {
            return Err(format!(
                "request pointer {} differs from the recorded context",
                self.pointer
            ));
        }
        Ok(())
    }

    fn apply(&self, request: &mut Value) -> Result<(), String> {
        self.check(request)?;
        if let Some(after) = &self.after {
            let slot = request
                .pointer_mut(self.pointer)
                .expect("checked request pointer");
            if let Some(range) = self.text_range {
                let Value::String(text) = slot else {
                    unreachable!("checked text fragment");
                };
                text.replace_range(range.bytes(), after.as_str().expect("text replacement"));
            } else {
                *slot = after.clone();
            }
        } else {
            let (parent, token) = self.pointer.rsplit_once('/').expect("validated pointer");
            match request.pointer_mut(parent) {
                Some(Value::Object(object)) => {
                    object.shift_remove(&token.replace("~1", "/").replace("~0", "~"));
                }
                Some(Value::Array(array)) => {
                    let index = token.parse::<usize>().expect("resolved array index");
                    array.remove(index);
                }
                _ => unreachable!("resolved non-root request pointer"),
            }
        }
        Ok(())
    }
}

fn validate_pair(first: &Intervention, second: &Intervention) -> Result<(), String> {
    validate_distinct_paths(first, second, "a pair test")
}

fn control_index<'a>(
    id: &str,
    edits: impl IntoIterator<Item = &'a Intervention>,
) -> Result<usize, String> {
    let mut edits = edits.into_iter();
    let (Some(first), Some(second), None) = (edits.next(), edits.next(), edits.next()) else {
        return Err("a negative control comparison requires exactly two interventions".into());
    };
    validate_distinct_paths(first, second, "a negative control comparison")?;
    match (first.context_id == id, second.context_id == id) {
        (true, false) => Ok(0),
        (false, true) => Ok(1),
        _ => Err("the negative control must identify one of the two edited context items".into()),
    }
}

fn validate_distinct_paths(
    first: &Intervention,
    second: &Intervention,
    design: &str,
) -> Result<(), String> {
    let disjoint_fragments = first.pointer == second.pointer
        && first
            .text_range
            .zip(second.text_range)
            .is_some_and(|(a, b)| a.disjoint(b));
    if !disjoint_fragments
        && (path_includes(&first.pointer, &second.pointer)
            || path_includes(&second.pointer, &first.pointer))
    {
        return Err(format!(
            "{design} requires non-overlapping request paths or text ranges"
        ));
    }
    Ok(())
}

fn path_includes(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn summarize(trials: &[Trial], matcher: &Matcher<'_>) -> Summary {
    let (mut completed, mut matched, mut unscored) = (0, 0, 0);
    for trial in trials {
        if let Trial::Completed { choice } = trial {
            completed += 1;
            match matcher.compare(choice) {
                Some(true) => matched += 1,
                Some(false) => {}
                None => unscored += 1,
            }
        }
    }
    Summary {
        attempted: trials.len(),
        completed,
        matched,
        unscored,
    }
}

fn difference(baseline: &Summary, variant: &Summary) -> Option<f64> {
    (baseline.matched > 0)
        .then(|| complete_difference(baseline, variant))
        .flatten()
}

fn complete_difference(baseline: &Summary, variant: &Summary) -> Option<f64> {
    if !baseline.all_scored() || !variant.all_scored() {
        return None;
    }
    Some(
        100.0
            * (variant.matched as f64 / variant.completed as f64
                - baseline.matched as f64 / baseline.completed as f64),
    )
}

fn interval(
    design: SamplingDesign,
    estimate: Option<f64>,
    samples_per_condition: usize,
    conditions: u8,
) -> Option<ReplayInterval> {
    let estimate = estimate?;
    if design.is_unspecified() {
        return None;
    }
    // A 95% two-sided Hoeffding bound. Each independent binary trial contributes
    // with weight ±1/n in the two- or four-condition contrast.
    let conditions = f64::from(conditions);
    let radius = 100.0 * (0.5 * 40_f64.ln() * conditions / samples_per_condition as f64).sqrt();
    let limit = 50.0 * conditions;
    Some(ReplayInterval {
        lower: (estimate - radius).max(-limit).floor() as i16,
        upper: (estimate + radius).min(limit).ceil() as i16,
    })
}

fn compare_control(
    control_index: usize,
    variants: &[Variant],
    design: SamplingDesign,
) -> ControlComparison {
    let control = &variants[control_index];
    let target = &variants[1 - control_index];
    let samples = target.summary.attempted;
    let contrast = (target.difference_pp.is_some() && control.difference_pp.is_some()).then(|| {
        // The common baseline cancels. Both variants have the same number of trials.
        let matches = target.summary.matched as i128 - control.summary.matched as i128;
        100.0 * matches as f64 / samples as f64
    });
    ControlComparison {
        control_context_id: control.intervention.context_id.clone(),
        target_minus_control_pp: contrast,
        target_minus_control_95_interval_pp: interval(design, contrast, samples, 2),
    }
}

fn interaction(baseline: &Summary, variants: &[Variant], pair: &Summary) -> Option<f64> {
    let [first, second] = variants else {
        return None;
    };
    (baseline.matched > 0)
        .then(|| complete_interaction(baseline, &first.summary, &second.summary, pair))
        .flatten()
}

fn complete_interaction(
    baseline: &Summary,
    first: &Summary,
    second: &Summary,
    pair: &Summary,
) -> Option<f64> {
    if [baseline, first, second, pair]
        .iter()
        .any(|summary| !summary.all_scored())
    {
        return None;
    }
    // All four conditions have the same number of trials. Contrast counts before dividing.
    let matches = pair.matched as i128 - first.matched as i128 - second.matched as i128
        + baseline.matched as i128;
    Some(100.0 * matches as f64 / baseline.attempted as f64)
}

fn locate<'a>(
    runs: impl IntoIterator<Item = &'a Run>,
    location: &Location,
) -> Result<(&'a crate::Call, Choice), String> {
    let call = runs
        .into_iter()
        .find(|run| run.id == location.run_id)
        .and_then(|run| run.calls.iter().find(|call| call.id == location.call_id))
        .ok_or("replay target call was not found")?;
    let decision = call
        .decisions
        .iter()
        .find(|decision| decision.id == location.decision_id)
        .ok_or("replay target decision was not found")?;
    Ok((
        call,
        Choice {
            kind: decision.kind.clone(),
            label: decision.label.clone(),
            outcome: decision.outcome.clone(),
        },
    ))
}

fn recorded_context<'a>(
    call: &'a crate::Call,
    context_id: &str,
) -> Result<(&'a ContextItem, &'a str), String> {
    call.context
        .iter()
        .find_map(|item| {
            if item.id != context_id || item.availability != Availability::Included {
                return None;
            }
            Some((item, item.text.as_deref()?))
        })
        .ok_or_else(|| format!("intervention context {context_id} has no captured text"))
}

fn prepare_edit<'a>(
    call: &crate::Call,
    intervention: &'a Intervention,
) -> Result<PreparedEdit<'a>, String> {
    let (item, text) = recorded_context(call, &intervention.context_id)?;
    let pointer = intervention.pointer.as_str();
    if !valid_pointer(pointer) {
        return Err(format!(
            "invalid JSON Pointer for {}",
            intervention.context_id
        ));
    }
    let text_range = match (
        &item.request_fragment,
        intervention.operation,
        intervention.text_range,
    ) {
        (Some(fragment), Operation::ReplaceTextFragment, Some(range))
            if fragment.pointer == pointer && fragment.text_range == range =>
        {
            fragment.validate()?;
            Some(range)
        }
        (None, operation, None) if operation != Operation::ReplaceTextFragment => None,
        _ => {
            return Err(format!(
                "request path and byte range for {} must match its declared fragment",
                intervention.context_id
            ))
        }
    };
    let (before, after, captured_context_sha256) = if intervention.operation.is_text()
        || text_range.is_some()
    {
        (
            Value::String(text.into()),
            Some(Value::String(intervention.replacement.clone())),
            sha256(text.as_bytes()),
        )
    } else {
        let before = captured_container(text).ok_or_else(|| {
            format!(
                "context {} has no captured JSON object or array",
                intervention.context_id
            )
        })?;
        let captured_context_sha256 = sha256(crate::canonical_json(&before).as_bytes());
        let after = match intervention.operation {
            Operation::ReplaceJson => {
                let value: Value =
                    serde_json::from_str(&intervention.replacement).map_err(|_| {
                        format!("invalid replacement JSON for {}", intervention.context_id)
                    })?;
                if before.is_object() != value.is_object() || before.is_array() != value.is_array()
                {
                    return Err(format!(
                        "replacement JSON must preserve the container type for {}",
                        intervention.context_id
                    ));
                }
                Some(value)
            }
            Operation::RemoveJson if intervention.replacement.is_empty() => None,
            _ => return Err(format!("invalid operation for {}", intervention.context_id)),
        };
        (before, after, captured_context_sha256)
    };
    if after.as_ref() == Some(&before) {
        return Err(format!(
            "intervention leaves context {} unchanged",
            intervention.context_id
        ));
    }
    Ok(PreparedEdit {
        pointer,
        text_range,
        before,
        after,
        captured_context_sha256,
    })
}
