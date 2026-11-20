//! Prepare exact requests for an external runner and score its recorded trials.

use super::{
    edited_requests, prepare_case, score_trials_with, sha256, Case, Location, Matcher,
    PreparedCase, Replay, Trial,
};
use crate::Run;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An experiment bound to the recorded decision and the exact request bytes.
#[derive(Debug, Serialize)]
pub struct Plan<'a> {
    pub version: u32,
    pub plan_id: String,
    pub case_id: &'a str,
    pub runner_id: &'a str,
    pub location: &'a Location,
    pub samples: usize,
    pub conditions: Vec<Condition<'a>>,
}

/// The baseline has no context IDs. Joint edits carry both IDs in case order.
#[derive(Debug, Serialize)]
pub struct Condition<'a> {
    pub context_ids: Vec<&'a str>,
    pub request_sha256: String,
    pub body: String,
}

/// One array of proposal outcomes per condition, in any condition order.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub version: u32,
    pub plan_id: String,
    pub conditions: Vec<ConditionTrials>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionTrials {
    pub context_ids: Vec<String>,
    pub request_sha256: String,
    pub trials: Vec<Trial>,
}

/// Validate a replay case and export its requests without invoking a runner.
pub fn prepare<'case, 'run>(
    case: &'case Case,
    runs: impl IntoIterator<Item = &'run Run>,
) -> Result<Plan<'case>, String> {
    let prepared = prepare_case(case, runs)?;
    Matcher::new(&prepared.match_rule, &prepared.recorded)?;
    plan(case, &prepared)
}

/// Rebuild the plan against the supplied evidence and score the submitted trials.
pub fn score<'run>(
    case: &Case,
    runs: impl IntoIterator<Item = &'run Run>,
    batch: Batch,
) -> Result<Replay, String> {
    let prepared = prepare_case(case, runs)?;
    score_trials_with(case, prepared, |prepared| {
        let plan = plan(case, prepared)?;
        if batch.version != 1 || batch.plan_id != plan.plan_id {
            return Err("The trial batch does not match the version 1 replay plan.".into());
        }
        let mut by_condition = BTreeMap::new();
        for result in batch.conditions {
            if by_condition
                .insert(result.context_ids, (result.request_sha256, result.trials))
                .is_some()
            {
                return Err("The trial batch repeats a replay condition.".into());
            }
        }
        let mut ordered = Vec::with_capacity(plan.conditions.len());
        for condition in plan.conditions {
            let key = condition
                .context_ids
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let (hash, trials) = by_condition
                .remove(&key)
                .ok_or("The trial batch is missing a replay condition.")?;
            if hash != condition.request_sha256 || trials.len() != plan.samples {
                return Err(
                    "A replay condition has a mismatched request digest or trial count.".into(),
                );
            }
            ordered.push(trials);
        }
        if !by_condition.is_empty() {
            return Err("The trial batch has an unexpected replay condition.".into());
        }
        Ok(ordered)
    })
}

fn plan<'case>(case: &'case Case, prepared: &PreparedCase<'_>) -> Result<Plan<'case>, String> {
    let requests = edited_requests(case.request.clone(), &prepared.edits, case.test_pair)?;
    let edited_ids = case
        .interventions
        .iter()
        .map(|edit| edit.context_id.as_str());
    let context_ids = std::iter::once(Vec::new())
        .chain(edited_ids.clone().map(|id| vec![id]))
        .chain(case.test_pair.then(|| edited_ids.collect()));
    let conditions: Vec<_> = requests
        .into_iter()
        .zip(context_ids)
        .map(|(request, context_ids)| {
            let body = request.to_string();
            Condition {
                context_ids,
                request_sha256: sha256(body.as_bytes()),
                body,
            }
        })
        .collect();
    let hashes: Vec<_> = conditions
        .iter()
        .map(|condition| condition.request_sha256.as_str())
        .collect();
    let fingerprint =
        serde_json::to_vec(&("nexusweave-replay-plan-1", case, &prepared.recorded, hashes))
            .map_err(|error| error.to_string())?;
    Ok(Plan {
        version: 1,
        plan_id: sha256(&fingerprint),
        case_id: &case.id,
        runner_id: &case.runner_id,
        location: &case.location,
        samples: case.samples,
        conditions,
    })
}
