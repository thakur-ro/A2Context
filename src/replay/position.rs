//! Execute the eight proposal conditions downloaded from a source-position comparison.

use super::{sha256, Choice, Location, MatchRule, Trial};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Experiment {
    SourcePosition,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Plan {
    pub version: u32,
    pub experiment: Experiment,
    pub pair: String,
    pub source: Value,
    /// Runner used for the earlier replays.
    pub runner_id: String,
    pub match_rule: MatchRule,
    pub backgrounds: [Background; 2],
    /// SHA-256 to compact serialized JSON. Several conditions can share one body.
    pub requests: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Background {
    pub location: Location,
    pub replay_id: String,
    pub context_id: String,
    pub pointer: String,
    pub original_position: usize,
    pub captured_request_sha256: String,
    pub conditions: [Condition; 4],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Condition {
    pub position: usize,
    pub source_replaced: bool,
    pub request_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Results {
    pub version: u32,
    pub pair: String,
    /// Identifies the runner that produced these proposals.
    pub runner_id: String,
    pub trials: Vec<Observation>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Observation {
    pub location: Location,
    pub replay_id: String,
    pub context_id: String,
    #[serde(flatten)]
    pub condition: Condition,
    pub trial: Trial,
}

impl Plan {
    fn inputs(&self) -> Result<BTreeMap<&str, Value>, String> {
        if self.version != 1
            || self.pair.len() != 64
            || !self.pair.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("expected a version 1 position plan and comparison digest".into());
        }
        let positions = self.backgrounds.each_ref().map(|b| b.original_position);
        if positions.contains(&0) || positions[0] == positions[1] {
            return Err("expected two distinct one-based source positions".into());
        }
        let mut inputs = BTreeMap::new();
        for (hash, body) in &self.requests {
            let request: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
            if !request.is_object()
                || sha256(body.as_bytes()) != *hash
                || serde_json::to_vec(&request).map_err(|e| e.to_string())? != body.as_bytes()
            {
                return Err("position plan contains a request with invalid bytes or digest".into());
            }
            inputs.insert(hash.as_str(), request);
        }
        let mut used = BTreeSet::new();
        for background in &self.backgrounds {
            let mut arms = BTreeSet::new();
            for arm in &background.conditions {
                if !inputs.contains_key(arm.request_sha256.as_str())
                    || !positions.contains(&arm.position)
                    || !arms.insert((arm.position, arm.source_replaced))
                {
                    return Err(
                        "position plan has a missing or duplicate condition or unknown request"
                            .into(),
                    );
                }
                if arm.position == background.original_position
                    && !arm.source_replaced
                    && arm.request_sha256 != background.captured_request_sha256
                {
                    return Err(
                        "original condition differs from the captured request digest".into(),
                    );
                }
                used.insert(&arm.request_sha256);
            }
        }
        if used.len() != inputs.len() {
            return Err("position plan has unused requests".into());
        }
        Ok(inputs)
    }

    /// Validate every body before asking the caller for one proposal per condition.
    pub fn execute(
        &self,
        runner_id: &str,
        mut sample: impl FnMut(&Value) -> Option<Choice>,
    ) -> Result<Results, String> {
        if runner_id.trim().is_empty() {
            return Err("identify the runner used for this experiment".into());
        }
        let inputs = self.inputs()?;
        let mut trials = Vec::with_capacity(8);
        for b in &self.backgrounds {
            for condition in &b.conditions {
                let answer = sample(&inputs[condition.request_sha256.as_str()]);
                trials.push(Observation {
                    location: b.location.clone(),
                    replay_id: b.replay_id.clone(),
                    context_id: b.context_id.clone(),
                    condition: condition.clone(),
                    trial: answer.map_or(Trial::Failed, |choice| Trial::Completed { choice }),
                });
            }
        }
        Ok(Results {
            version: 1,
            pair: self.pair.clone(),
            runner_id: runner_id.to_owned(),
            trials,
        })
    }
}

/// Counts of backgrounds with complete proposal pairs at both positions.
#[derive(Default, Serialize)]
pub struct Contrast {
    pub compared: usize,
    pub changed: usize,
}

impl Plan {
    /// Check the eight submitted trial bindings against this plan. Count full
    /// proposal differences only when both positions returned a proposal.
    pub fn verify(&self, result: &Results) -> Result<[Contrast; 2], String> {
        if result.version != 1
            || result.pair != self.pair
            || result.runner_id.trim().is_empty()
            || result.trials.len() != 8
        {
            return Err("expected eight trials for the selected version 1 comparison".into());
        }
        let mut contrasts = [Contrast::default(), Contrast::default()];
        for b in &self.backgrounds {
            let mut choices = [Vec::new(), Vec::new()];
            for condition in &b.conditions {
                let mut observations = result.trials.iter().filter(|r| {
                    r.location == b.location
                        && r.replay_id == b.replay_id
                        && r.context_id == b.context_id
                        && r.condition == *condition
                });
                let observation = observations
                    .next()
                    .ok_or("missing trial or mismatched request digest")?;
                if observations.next().is_some() {
                    return Err("duplicate position trial".into());
                }
                if let Trial::Completed { choice } = &observation.trial {
                    choices[usize::from(condition.source_replaced)].push(choice);
                }
            }
            for (contrast, pair) in contrasts.iter_mut().zip(choices) {
                if let [first, second] = pair.as_slice() {
                    contrast.compared += 1;
                    contrast.changed += usize::from(first != second);
                }
            }
        }
        Ok(contrasts)
    }
}
