//! Compare recorded decisions without pairing parallel calls by position.

use crate::Decision;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    DifferentKindOrLabel,
    Unavailable,
    Same,
    Different,
}

pub fn compare_outcomes(left: &Decision, right: &Decision) -> OutcomeStatus {
    if left.kind != right.kind || left.label != right.label {
        OutcomeStatus::DifferentKindOrLabel
    } else if left.outcome.is_null() || right.outcome.is_null() {
        OutcomeStatus::Unavailable
    } else if left.outcome == right.outcome {
        OutcomeStatus::Same
    } else {
        OutcomeStatus::Different
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionChangeStatus {
    Same,
    Incomplete,
    OutcomeChanged,
    MixChanged,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct DecisionChange<'a> {
    pub status: DecisionChangeStatus,
    pub groups: Vec<DecisionGroup<'a>>,
}

#[derive(Debug, Serialize)]
pub struct DecisionGroup<'a> {
    pub kind: &'a str,
    pub label: &'a str,
    pub decision_ids: [Vec<&'a str>; 2],
    pub status: DecisionChangeStatus,
}

pub fn compare_decisions<'a>(before: &'a [Decision], after: &'a [Decision]) -> DecisionChange<'a> {
    let mut grouped = BTreeMap::<_, [Vec<_>; 2]>::new();
    for (side, decisions) in [before, after].into_iter().enumerate() {
        for decision in decisions {
            grouped
                .entry((decision.kind.as_str(), decision.label.as_str()))
                .or_default()[side]
                .push(decision);
        }
    }
    let unavailable = before.is_empty() || after.is_empty();
    let groups: Vec<_> = grouped
        .into_iter()
        .map(|((kind, label), decisions)| DecisionGroup {
            kind,
            label,
            status: if unavailable {
                DecisionChangeStatus::Unavailable
            } else {
                compare_group(&decisions[0], &decisions[1])
            },
            decision_ids: decisions
                .map(|decisions| decisions.into_iter().map(|item| item.id.as_str()).collect()),
        })
        .collect();
    // The enum orders the evidence from a match to the strongest visible change.
    let status = groups
        .iter()
        .map(|group| group.status)
        .max()
        .unwrap_or(DecisionChangeStatus::Unavailable);
    DecisionChange { status, groups }
}

fn compare_group(before: &[&Decision], after: &[&Decision]) -> DecisionChangeStatus {
    use DecisionChangeStatus as Change;
    if before.len() != after.len() {
        return Change::MixChanged;
    }
    if let ([left], [right]) = (before, after) {
        return match compare_outcomes(left, right) {
            OutcomeStatus::Same => Change::Same,
            OutcomeStatus::Different => Change::OutcomeChanged,
            OutcomeStatus::Unavailable => Change::Incomplete,
            OutcomeStatus::DifferentKindOrLabel => {
                unreachable!("decisions grouped by kind and label")
            }
        };
    }
    let mut unmatched = HashMap::<&Value, usize>::new();
    let mut missing_before = 0;
    let mut missing_after = 0;
    let mut unmatched_after = 0;
    for decision in before {
        if decision.outcome.is_null() {
            missing_before += 1;
        } else {
            *unmatched.entry(&decision.outcome).or_default() += 1;
        }
    }
    for decision in after {
        if decision.outcome.is_null() {
            missing_after += 1;
        } else if let Some(count) = unmatched
            .get_mut(&decision.outcome)
            .filter(|count| **count > 0)
        {
            *count -= 1;
        } else {
            unmatched_after += 1;
        }
    }
    // Missing outcomes can account for at most one unmatched recorded value each.
    if unmatched_after > missing_before || unmatched.values().sum::<usize>() > missing_after {
        Change::OutcomeChanged
    } else if missing_before > 0 || missing_after > 0 {
        Change::Incomplete
    } else {
        Change::Same
    }
}
