//! Bind optional decoder-block measurements to verified candidate conditions.

use super::{ConditionScore, Report};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TOLERANCE: f64 = 0.0001;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    version: u32,
    model_id: String,
    model_revision: String,
    runtime: String,
    tokenizer_sha256: String,
    blocks: usize,
    choice_token_ids: [u32; 2],
    baseline: Condition,
    self_swaps: Sites<Vec<f64>>,
    effects: Vec<Effect>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    request_sha256: String,
    rendered_prefix_sha256: String,
    token_count: usize,
    positions: Sites<Position>,
    margin: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sites<T> {
    last_user_token: T,
    last_prefix_token: T,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Position {
    index: usize,
    token_id: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Effect {
    condition_id: String,
    donor: Condition,
    swaps: Sites<Vec<f64>>,
}

pub(super) fn review(report: &Report) -> Option<Value> {
    let raw = report.block_probe.as_ref()?;
    let Ok(probe) = Probe::deserialize(raw) else {
        return Some(
            json!({"status":"unverified","issue":"The block probe has an invalid format."}),
        );
    };
    Some(match probe.validate(report) {
        Ok(()) => json!({"status":"matched","probe":probe}),
        Err(issue) => json!({"status":"unverified","issue":issue}),
    })
}

impl Probe {
    fn validate(&self, report: &Report) -> Result<(), &'static str> {
        let scores = &report.scores;
        let prefix_token = *scores
            .assistant_prefix_token_ids
            .last()
            .ok_or("Block probes require a supplied answer prefix.")?;
        let choices = scores.candidate_token_ids.each_ref().map(|ids| ids[0]);
        let single_tokens = scores.candidate_token_ids.iter().all(|ids| ids.len() == 1);
        if self.version != 1
            || !(2..=256).contains(&self.blocks)
            || self.model_id != scores.model_id
            || self.model_revision != scores.model_revision
            || self.runtime != scores.runtime
            || self.tokenizer_sha256 != scores.tokenizer_sha256
            || choices[0] == choices[1]
            || self.choice_token_ids != choices
            || self.effects.len() != report.effects.len()
        {
            return Err(
                "The block probe must match the model, first candidate tokens and source edits.",
            );
        }
        if !self.baseline.matches(
            &report.captured_request_sha256,
            &scores.conditions[0],
            prefix_token,
            single_tokens,
        ) {
            return Err("The block probe baseline differs from the scored request or prefix.");
        }
        let original = self.baseline.margin;
        if !self.self_swaps.complete(self.blocks)
            || self.self_swaps.values().any(|v| !close(v, original))
        {
            return Err(
                "The self-swaps must reproduce the original margin at every block and site.",
            );
        }
        for ((effect, scored), condition) in self
            .effects
            .iter()
            .zip(&report.effects)
            .zip(&scores.conditions[1..])
        {
            if effect.condition_id != scored.condition_id
                || !effect.donor.matches(
                    &scored.request_sha256,
                    condition,
                    prefix_token,
                    single_tokens,
                )
                || effect.donor.positions.last_user_token.token_id
                    != self.baseline.positions.last_user_token.token_id
            {
                return Err(
                    "A donor must match the source edit, rendered prefix and aligned tokens.",
                );
            }
            if !effect.swaps.complete(self.blocks)
                || !close(effect.swaps.last_user_token[self.blocks - 1], original)
                || !close(
                    effect.swaps.last_prefix_token[self.blocks - 1],
                    effect.donor.margin,
                )
            {
                return Err("The block sweep is incomplete or the final-block controls failed.");
            }
        }
        Ok(())
    }
}

impl Condition {
    fn matches(
        &self,
        hash: &str,
        score: &ConditionScore,
        prefix_token: u32,
        single_tokens: bool,
    ) -> bool {
        let margin = match &score.steps {
            Some([a, b]) => Some(a[0].log_prob - b[0].log_prob),
            None if single_tokens => Some(super::margin(score)),
            None => None,
        };
        self.request_sha256 == hash
            && self.rendered_prefix_sha256 == score.rendered_prefix_sha256
            && self.positions.last_prefix_token.index.checked_add(1) == Some(self.token_count)
            && self.positions.last_user_token.index < self.positions.last_prefix_token.index
            && self.positions.last_prefix_token.token_id == prefix_token
            && self.margin.is_finite()
            && margin.is_none_or(|value| close(self.margin, value))
    }
}

impl Sites<Vec<f64>> {
    fn values(&self) -> impl Iterator<Item = f64> + '_ {
        self.last_user_token
            .iter()
            .chain(&self.last_prefix_token)
            .copied()
    }

    fn complete(&self, blocks: usize) -> bool {
        self.last_user_token.len() == blocks
            && self.last_prefix_token.len() == blocks
            && self.values().all(f64::is_finite)
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE
}
