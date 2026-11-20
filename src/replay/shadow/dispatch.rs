//! Compare an identical tool proposal on two copies of a sandbox checkpoint.

use super::{Choice, Effect, Report, Trial};
use crate::{canonical_json, replay::sha256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

type State = BTreeMap<String, Value>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Comparison {
    version: u32,
    edited_request_sha256: String,
    runner_id: String,
    guard_id: String,
    proposal: Choice,
    checkpoint: State,
    target_key: String,
    expected: Value,
    without_guard: Arm,
    with_guard: Arm,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Arm {
    checkpoint_sha256: String,
    status: Status,
    after: State,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Applied,
    Blocked,
}

/// Project validated dispatch evidence for a verified shadow edit.
///
/// The producer supplies the checkpoint and outcomes. Invalid optional
/// measurements stay separate from the shadow proposal comparison.
pub fn review(report: &Report, effect: &Effect) -> Option<Value> {
    let raw = report.sandbox_dispatch.as_ref()?;
    if raw.get("edited_request_sha256")?.as_str()? != effect.request_sha256 {
        return None;
    }
    let comparison = Comparison::deserialize(raw).ok()?;
    let protected = comparison.validate(report, effect)?;
    Some(json!({
        "runner":comparison.runner_id, "guard":comparison.guard_id,
        "checkpoint":comparison.with_guard.checkpoint_sha256,
        "target":comparison.target_key, "protected":protected,
        "without_guard":comparison.without_guard.status,
        "with_guard":comparison.with_guard.status,
        "states":comparison.checkpoint.iter().map(|(key,before)| json!({
            "key":key, "before":before, "without_guard":comparison.without_guard.after[key],
            "with_guard":comparison.with_guard.after[key],
        })).collect::<Vec<_>>()
    }))
}

impl Comparison {
    fn validate(&self, report: &Report, effect: &Effect) -> Option<bool> {
        if self.version != 1
            || self.proposal.kind != "tool_call"
            || report
                .effects
                .iter()
                .filter(|arm| arm.request_sha256 == self.edited_request_sha256)
                .take(2)
                .count()
                != 1
            || [&self.runner_id, &self.guard_id]
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > 128)
            || !identical(&report.baseline, &self.proposal)
            || !identical(&effect.result.trials, &self.proposal)
            || !valid_state(&self.checkpoint)
        {
            return None;
        }
        let checkpoint = canonical_json(&serde_json::to_value(&self.checkpoint).ok()?);
        let digest = sha256(checkpoint.as_bytes());
        for arm in [&self.without_guard, &self.with_guard] {
            if arm.checkpoint_sha256 != digest
                || !arm.after.keys().eq(self.checkpoint.keys())
                || !valid_state(&arm.after)
                || (arm.status == Status::Blocked && arm.after != self.checkpoint)
            {
                return None;
            }
        }
        let before = self.checkpoint.get(&self.target_key)?;
        Some(
            before == &self.expected
                && self.without_guard.status == Status::Applied
                && self.with_guard.status == Status::Blocked
                && self.without_guard.after[&self.target_key] != self.expected,
        )
    }
}

fn identical(trials: &[Trial], original: &Choice) -> bool {
    !trials.is_empty()
        && trials
            .iter()
            .all(|trial| matches!(trial, Trial::Completed { choice } if choice == original))
}

fn valid_state(state: &State) -> bool {
    (1..=12).contains(&state.len())
        && state.iter().all(|(name, value)| {
            !name.trim().is_empty() && name.len() <= 128 && value.to_string().len() <= 256
        })
}
