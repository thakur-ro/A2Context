//! Attach measured decoder-block interventions to their recorded workflow decisions.

pub mod generation_branches;
pub mod output_scores;
pub mod patterns;
pub(crate) mod source_swaps;
pub mod visibility;

use crate::{replay::Location, Availability, Call, Decision, Run};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum Report {
    Point(PointReport),
    Timeline(TimelineReport),
    Visibility(visibility::Report),
}

pub(crate) fn decision_path(location: &Location) -> String {
    decision_view_path(
        &location.run_id,
        &location.call_id,
        Some(&location.decision_id),
    )
}

pub(crate) fn decision_view_path(run_id: &str, call_id: &str, decision_id: Option<&str>) -> String {
    let run = utf8_percent_encode(run_id, NON_ALPHANUMERIC);
    let call = utf8_percent_encode(call_id, NON_ALPHANUMERIC);
    let mut path = format!("/#v=1&view=decisions&run={run}&call={call}");
    if let Some(id) = decision_id {
        let decision = utf8_percent_encode(id, NON_ALPHANUMERIC);
        path.push_str(&format!("&decision={decision}"));
    }
    path
}

fn activation_path(
    location: &Location,
    report_id: &str,
    position: &str,
    index: usize,
    block: usize,
) -> String {
    let path = activation_report_path(location, report_id);
    format!("{path}&{position}={index}&block={block}")
}

fn activation_report_path(location: &Location, report_id: &str) -> String {
    let path = decision_path(location);
    let report = utf8_percent_encode(report_id, NON_ALPHANUMERIC);
    format!("{path}&activation={report}")
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PointReport {
    version: u32,
    id: String,
    location: Location,
    case_id: String,
    model_id: String,
    model_revision: String,
    runtime: String,
    prompt_sha256: String,
    tokenizer_sha256: String,
    readout: Readout,
    alternatives: [String; 2],
    choice_token_ids: [u32; 2],
    prediction_position: usize,
    layer_count: usize,
    baseline: Outcome,
    checks: Checks,
    sources: Vec<Source<Outcome>>,
    sweeps: Vec<Sweep>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineReport {
    version: u32,
    id: String,
    location: Location,
    case_id: String,
    model_id: String,
    model_revision: String,
    runtime: String,
    prompt_sha256: String,
    tokenizer_sha256: String,
    readout: Readout,
    layer_count: usize,
    baseline_answer: String,
    prompt_token_count: usize,
    output_token_ids: Vec<u32>,
    sources: Vec<Source<String>>,
    checkpoints: Vec<Checkpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<ToolCalls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_scores: Option<Box<output_scores::OutputScores>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation_branches: Option<Vec<generation_branches::Branch>>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ToolCalls {
    baseline: ToolProposal,
    edited: BTreeMap<String, Option<ToolProposal>>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ToolProposal {
    name: String,
    arguments: Map<String, Value>,
}

impl ToolProposal {
    fn reproduces(&self, decision: &Decision) -> bool {
        decision.kind == "tool_call"
            && !self.name.trim().is_empty()
            && self.name == decision.label
            && decision.outcome.as_object() == Some(&self.arguments)
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    label: String,
    output_token: usize,
    answer_prefix: String,
    alternatives: [String; 2],
    choice_token_ids: [u32; 2],
    baseline_margin: f64,
    edited_margins: BTreeMap<String, f64>,
    checks: Checks,
    sweeps: Vec<Sweep>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Readout {
    NextTokenLogOdds,
    OutputTokenLogProbability,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Outcome {
    answer: i64,
    margin: f64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Checks {
    layer: usize,
    source_context_id: String,
    clean_patch_margin: f64,
    full_donor_margin: f64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Source<T> {
    context_id: String,
    captured_text_sha256: String,
    token_range: [usize; 2],
    edited_token_range: [usize; 2],
    before: String,
    after: String,
    edited_outcome: T,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Sweep {
    source_context_id: String,
    target: Target,
    token_range: [usize; 2],
    margins: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Target {
    SourceTokens,
    PredictionToken,
}

impl Report {
    pub fn has_checkpoints(&self) -> bool {
        matches!(self, Self::Timeline(report) if !report.checkpoints.is_empty())
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Point(report) => &report.id,
            Self::Timeline(report) => &report.id,
            Self::Visibility(report) => &report.id,
        }
    }

    pub fn location(&self) -> &Location {
        match self {
            Self::Point(report) => &report.location,
            Self::Timeline(report) => &report.location,
            Self::Visibility(report) => &report.location,
        }
    }

    pub fn validate(&self, runs: &[Run]) -> Result<(), String> {
        let (call, decision) = find_decision(runs, self.location())?;
        match self {
            Self::Point(report) => report.validate(call, decision),
            Self::Timeline(report) => report.validate(call, decision),
            Self::Visibility(report) => report.validate(call, decision),
        }
    }
}

impl PointReport {
    fn validate(&self, call: &Call, decision: &Decision) -> Result<(), String> {
        if self.version != 1
            || !matches!(self.readout, Readout::NextTokenLogOdds)
            || !has_provenance(
                [
                    &self.id,
                    &self.case_id,
                    &self.model_id,
                    &self.model_revision,
                    &self.runtime,
                ],
                [&self.prompt_sha256, &self.tokenizer_sha256],
            )
        {
            return Err(
                "expected a versioned activation report with model and prompt provenance".into(),
            );
        }
        if !valid_contrast(&self.alternatives, self.choice_token_ids)
            || !self
                .alternatives
                .contains(&self.baseline.answer.to_string())
        {
            return Err(
                "activation report alternatives must include the reproduced decision".into(),
            );
        }
        let prefix_length = self
            .prediction_position
            .checked_add(1)
            .ok_or("the activation prediction position is outside the supported range")?;
        if decision.outcome.as_i64() != Some(self.baseline.answer) {
            return Err("the activation baseline did not reproduce the recorded decision".into());
        }
        let sources = validate_sources(&self.sources, call, prefix_length)?;
        validate_sweeps(
            &self.sweeps,
            &sources,
            self.layer_count,
            prefix_length,
            self.baseline.margin,
        )?;
        let donor = sources
            .get(self.checks.source_context_id.as_str())
            .ok_or("the full-donor control references an unknown source edit")?;
        self.checks.validate(
            self.layer_count,
            self.baseline.margin,
            donor.edited_outcome.margin,
        )
    }
}

impl TimelineReport {
    fn validate(&self, call: &Call, decision: &Decision) -> Result<(), String> {
        if self.version != 2
            || !has_provenance(
                [
                    &self.id,
                    &self.case_id,
                    &self.model_id,
                    &self.model_revision,
                    &self.runtime,
                ],
                [&self.prompt_sha256, &self.tokenizer_sha256],
            )
        {
            return Err(
                "expected a versioned activation timeline with model and prompt provenance".into(),
            );
        }
        let reproduced = match (&self.tool_calls, decision.kind.as_str()) {
            (Some(calls), "tool_call") => calls.baseline.reproduces(decision),
            (None, kind) if kind != "tool_call" => {
                decision.outcome.as_str() == Some(&self.baseline_answer)
            }
            _ => false,
        };
        if self.baseline_answer.is_empty() || !reproduced {
            return Err("the activation baseline did not reproduce the recorded decision".into());
        }
        let has_scores = self.output_scores.is_some();
        let valid_readout = matches!(
            (&self.readout, self.checkpoints.len(), has_scores),
            (Readout::NextTokenLogOdds, 2.., _)
                | (Readout::NextTokenLogOdds, 1, true)
                | (Readout::OutputTokenLogProbability, 0, true)
        );
        if !valid_readout || self.prompt_token_count == 0 || self.output_token_ids.is_empty() {
            return Err(
                "an activation timeline needs matching measurements for its declared readout"
                    .into(),
            );
        }
        let sources = validate_sources(&self.sources, call, self.prompt_token_count)?;
        if let Some(branches) = &self.generation_branches {
            generation_branches::validate(self, branches, sources.keys().copied().collect())?;
        }
        if let Some(scores) = &self.output_scores {
            scores.validate(self, sources.keys().copied())?;
        }
        if let Some(calls) = &self.tool_calls {
            if !sources
                .keys()
                .copied()
                .eq(calls.edited.keys().map(String::as_str))
                || calls
                    .edited
                    .values()
                    .flatten()
                    .any(|proposal| proposal.name.trim().is_empty())
            {
                return Err("the activation report needs a tool proposal or a text fallback for each source edit".into());
            }
        }
        let mut previous: Option<&Checkpoint> = None;
        for step in &self.checkpoints {
            let prefix_length = self
                .prompt_token_count
                .checked_add(step.output_token)
                .ok_or("the activation prediction position is outside the supported range")?;
            let selected = self.baseline_answer.strip_prefix(&step.answer_prefix);
            if previous.is_some_and(|prior| {
                prior.output_token >= step.output_token
                    || prior.answer_prefix.len() >= step.answer_prefix.len()
            }) || !valid_contrast(&step.alternatives, step.choice_token_ids)
                || self.output_token_ids.get(step.output_token) != Some(&step.choice_token_ids[0])
                || !selected
                    .is_some_and(|suffix| suffix.trim_start().starts_with(&step.alternatives[0]))
                || step.label.trim().is_empty()
            {
                return Err("an activation checkpoint is out of order or does not match the baseline answer".into());
            }
            if !sources
                .keys()
                .copied()
                .eq(step.edited_margins.keys().map(String::as_str))
            {
                return Err(
                    "an activation checkpoint must provide an edited margin for each source".into(),
                );
            }
            let donor = step
                .edited_margins
                .get(&step.checks.source_context_id)
                .ok_or("the full-donor control references an unknown source edit")?;
            step.checks
                .validate(self.layer_count, step.baseline_margin, *donor)?;
            let sites = validate_sweeps(
                &step.sweeps,
                &sources,
                self.layer_count,
                prefix_length,
                step.baseline_margin,
            )?;
            if !sources
                .keys()
                .all(|id| sites.contains(&(*id, Target::SourceTokens)))
            {
                return Err(
                    "an activation checkpoint must sweep the edited tokens of each source".into(),
                );
            }
            previous = Some(step);
        }
        Ok(())
    }
}

impl Checks {
    fn validate(&self, layer_count: usize, baseline: f64, donor: f64) -> Result<(), String> {
        if self.layer >= layer_count {
            return Err("activation control layer is outside the measured model".into());
        }
        if (self.clean_patch_margin - baseline).abs() > 0.05
            || (self.full_donor_margin - donor).abs() > 0.05
        {
            return Err("the activation report failed its instrumentation controls".into());
        }
        Ok(())
    }
}

fn find_decision<'a>(
    runs: &'a [Run],
    location: &Location,
) -> Result<(&'a Call, &'a Decision), String> {
    let call = runs
        .iter()
        .find(|run| run.id == location.run_id)
        .and_then(|run| run.calls.iter().find(|call| call.id == location.call_id))
        .ok_or("the activation report call was not found")?;
    let decision = call
        .decisions
        .iter()
        .find(|item| item.id == location.decision_id)
        .ok_or("the activation report decision was not found")?;
    Ok((call, decision))
}

fn validate_sources<'a, T>(
    entries: &'a [Source<T>],
    call: &Call,
    prompt_length: usize,
) -> Result<BTreeMap<&'a str, &'a Source<T>>, String> {
    let mut sources = BTreeMap::new();
    for source in entries {
        let text = call
            .context
            .iter()
            .find(|item| {
                item.id == source.context_id && item.availability == Availability::Included
            })
            .and_then(|item| item.text.as_deref())
            .ok_or("an activation source has no captured text on this call")?;
        if format!("{:x}", Sha256::digest(text.as_bytes())) != source.captured_text_sha256 {
            return Err("activation source text differs from the recorded context".into());
        }
        if source.before.is_empty()
            || source.before == source.after
            || !text.contains(&source.before)
            || !inside(source.token_range, [0, prompt_length])
            || !inside(source.edited_token_range, source.token_range)
            || sources.insert(source.context_id.as_str(), source).is_some()
        {
            return Err("an activation source has an invalid edit or token range".into());
        }
    }
    Ok(sources)
}

fn validate_sweeps<'a, T>(
    sweeps: &'a [Sweep],
    sources: &BTreeMap<&str, &Source<T>>,
    layer_count: usize,
    prefix_length: usize,
    baseline: f64,
) -> Result<BTreeSet<(&'a str, Target)>, String> {
    let mut sites = BTreeSet::new();
    for sweep in sweeps {
        let source = sources
            .get(sweep.source_context_id.as_str())
            .ok_or("an activation sweep references an unknown source edit")?;
        let expected = match sweep.target {
            Target::SourceTokens => source.edited_token_range,
            Target::PredictionToken => [prefix_length - 1, prefix_length],
        };
        if sweep.token_range != expected
            || sweep.margins.len() != layer_count
            || !sites.insert((sweep.source_context_id.as_str(), sweep.target))
        {
            return Err(
                "an activation sweep has invalid positions or a mismatched layer count".into(),
            );
        }
        if sweep
            .margins
            .iter()
            .any(|score| !(score - baseline).is_finite())
        {
            return Err("an activation sweep contains a non-finite change in log odds".into());
        }
    }
    if sites.is_empty() {
        return Err("the activation report has no measured sites".into());
    }
    Ok(sites)
}

fn has_provenance(fields: [&str; 5], digests: [&str; 2]) -> bool {
    fields.iter().all(|value| !value.trim().is_empty()) && digests.into_iter().all(is_digest)
}

fn valid_contrast(alternatives: &[String; 2], token_ids: [u32; 2]) -> bool {
    alternatives.iter().all(|value| !value.trim().is_empty())
        && alternatives[0] != alternatives[1]
        && token_ids[0] != token_ids[1]
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn inside([start, end]: [usize; 2], [lower, upper]: [usize; 2]) -> bool {
    lower <= start && start < end && end <= upper
}
