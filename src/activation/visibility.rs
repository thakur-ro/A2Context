//! Measure changes in recorded token scores when a prompt source is masked.

pub mod patterns;
pub mod query;

use super::{inside, is_digest, ToolProposal};
use crate::{
    replay::{captured_container, valid_pointer, Location},
    Availability, Call, Decision,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    version: u32,
    kind: Kind,
    pub(super) id: String,
    pub(super) location: Location,
    model_id: String,
    model_revision: String,
    runtime: String,
    prompt_sha256: String,
    tokenizer_sha256: String,
    request_sha256: String,
    prompt_token_count: usize,
    baseline_answer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline_tool_call: Option<ToolProposal>,
    output_token_ids: Vec<u32>,
    fragments: Vec<String>,
    baseline_log_probs: Vec<f64>,
    all_visible_max_abs_error: f64,
    sources: Vec<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pairs: Vec<Pair>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Target {
    key: String,
    token_range: [usize; 2],
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    SourceVisibility,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
enum Source {
    Measured(Measurement),
    Skipped(Skipped),
}

impl Source {
    fn id(&self) -> &str {
        match self {
            Self::Measured(source) => &source.context_id,
            Self::Skipped(source) => &source.context_id,
        }
    }

    fn measured(&self) -> Option<&Measurement> {
        match self {
            Self::Measured(source) => Some(source),
            Self::Skipped(_) => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Measurement {
    context_id: String,
    captured_text_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_pointer: Option<String>,
    token_range: [usize; 2],
    masked_log_probs: Vec<f64>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Pair {
    context_ids: [String; 2],
    masked_log_probs: Vec<f64>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Skipped {
    context_id: String,
    unavailable: Unavailable,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Unavailable {
    TextNotCaptured,
    NoRequestFragment,
    UnsupportedRequestField,
    RequestObjectNotFound,
    RequestObjectAmbiguous,
    RenderedSourceUnaligned,
    TokenBoundaryUnaligned,
}

impl Report {
    pub(super) fn validate(&self, call: &Call, decision: &Decision) -> Result<(), String> {
        let fields = [
            &self.id,
            &self.model_id,
            &self.model_revision,
            &self.runtime,
        ];
        if self.version != 1
            || fields.iter().any(|field| field.trim().is_empty())
            || !is_digest(&self.prompt_sha256)
            || !is_digest(&self.tokenizer_sha256)
        {
            return Err(
                "expected source visibility report version 1 with model and prompt provenance"
                    .into(),
            );
        }
        let recorded_request = call
            .recorded_request
            .as_ref()
            .ok_or("source visibility reports require the recorded request")?;
        if digest(&recorded_request.body) != self.request_sha256 {
            return Err(
                "source visibility request digest differs from the recorded request".into(),
            );
        }
        let reproduced = match (&self.baseline_tool_call, decision.kind.as_str()) {
            (Some(proposal), "tool_call") => proposal.reproduces(decision),
            (None, "model_output") => decision.outcome.as_str() == Some(&self.baseline_answer),
            _ => false,
        };
        if self.baseline_answer.is_empty() || !reproduced {
            return Err(
                "the source visibility baseline did not reproduce the recorded decision".into(),
            );
        }
        let length = self.output_token_ids.len();
        if self.prompt_token_count < 2
            || length == 0
            || self.fragments.len() != length
            || self.fragments.concat() != self.baseline_answer
            || !log_probs(&self.baseline_log_probs, length)
            || !(0.0..=1e-5).contains(&self.all_visible_max_abs_error)
        {
            return Err(
                "source visibility needs aligned token scores and a passing all-visible control"
                    .into(),
            );
        }
        let mut target_keys = BTreeSet::new();
        for target in &self.targets {
            if target.key.trim().is_empty()
                || target.key.trim() != target.key
                || !target_keys.insert(&target.key)
                || !inside(target.token_range, [0, length])
            {
                return Err(
                    "output targets require unique keys and nonempty recorded token ranges".into(),
                );
            }
        }
        let request = recorded_request.parse()?;
        let context: BTreeMap<_, _> = call
            .context
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect();
        let mut seen = BTreeSet::new();
        let mut measured = BTreeMap::new();
        for source in &self.sources {
            let id = source.id();
            let item = context
                .get(id)
                .ok_or("a source visibility entry references unknown context")?;
            if !seen.insert(id) {
                return Err("source visibility reports may only list each input once".into());
            }
            if let Source::Measured(source) = source {
                let text = item
                    .text
                    .as_deref()
                    .filter(|_| item.availability == Availability::Included)
                    .filter(|text| match (&item.request_fragment, &source.request_pointer) {
                        (Some(fragment), None) => fragment.text(&request) == Some(*text),
                        (None, Some(pointer)) if valid_pointer(pointer) => {
                            request.pointer(pointer).is_some_and(|value| {
                                captured_container(text).as_ref() == Some(value)
                            })
                        }
                        _ => false,
                    })
                    .ok_or(
                        "a measured source needs text aligned to its recorded request fragment or JSON value",
                    )?;
                if digest(text) != source.captured_text_sha256
                    || !inside(source.token_range, [0, self.prompt_token_count - 1])
                    || !log_probs(&source.masked_log_probs, length)
                {
                    return Err("a measured source has invalid text, positions, or scores".into());
                }
                measured.insert(id, source);
            }
        }
        if measured.is_empty() || seen.len() != context.len() {
            return Err(
                "source visibility reports must record coverage for every captured input".into(),
            );
        }
        self.validate_pairs(&measured)?;
        Ok(())
    }

    fn validate_pairs(&self, measured: &BTreeMap<&str, &Measurement>) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for pair in &self.pairs {
            let mut keys = pair.context_ids.each_ref().map(String::as_str);
            keys.sort_unstable();
            let [Some(first), Some(second)] = keys.map(|key| measured.get(key)) else {
                return Err("a joint mask requires two individually measured sources".into());
            };
            let [a, b] = [first.token_range, second.token_range];
            if a[1] > b[0] && b[1] > a[0] {
                return Err("a joint mask requires distinct, non-overlapping source tokens".into());
            }
            if !seen.insert(keys) {
                return Err("a joint mask may only be recorded once per source pair".into());
            }
            if !log_probs(&pair.masked_log_probs, self.baseline_log_probs.len())
                || pair
                    .masked_log_probs
                    .iter()
                    .enumerate()
                    .any(|(index, joint)| {
                        let first = first.masked_log_probs[index];
                        let second = second.masked_log_probs[index];
                        let baseline = self.baseline_log_probs[index];
                        !interaction(baseline, first, second, *joint).is_finite()
                    })
            {
                return Err("a joint mask has invalid or misaligned token scores".into());
            }
        }
        Ok(())
    }
}

fn interaction(baseline: f64, first: f64, second: f64, joint: f64) -> f64 {
    (joint - first) - (second - baseline)
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn log_probs(values: &[f64], length: usize) -> bool {
    values.len() == length
        && values
            .iter()
            .all(|value| value.is_finite() && *value <= 0.0)
}
