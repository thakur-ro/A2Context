//! Compare observed outcomes across conditions within a saved replay.

use super::{
    matching::{Matcher, Projection},
    MatchRule, Replay, Trial,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Serialize)]
pub struct OutcomeDistribution<'a> {
    pub replay_index: usize,
    pub buckets: Vec<Bucket<'a>>,
}

#[derive(Debug, Serialize)]
pub struct Bucket<'a> {
    pub status: Status,
    pub example: TrialPosition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternative_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection<'a>>,
    pub counts: Vec<ConditionCount>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Match,
    Alternative,
    Unscored,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Selection<'a> {
    Fields(BTreeMap<String, Value>),
    Capture(&'a str),
}

#[derive(Debug, Serialize)]
pub struct TrialPosition {
    pub condition: usize,
    pub trial: usize,
}

#[derive(Debug, Serialize)]
pub struct ConditionCount {
    pub condition: usize,
    pub count: usize,
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Key<'a> {
    Scored {
        kind: &'a str,
        label: &'a str,
        projection: Projection<'a>,
    },
    Unscored,
    Failed,
}

struct Occurrences {
    example: TrialPosition,
    counts: Vec<ConditionCount>,
}

pub fn outcome_distributions(reports: &[Replay]) -> Result<Vec<OutcomeDistribution<'_>>, String> {
    let mut alternatives = BTreeMap::new();
    reports
        .iter()
        .enumerate()
        .map(|(replay_index, report)| {
            Ok(OutcomeDistribution {
                replay_index,
                buckets: outcomes(report, &mut alternatives)?,
            })
        })
        .collect()
}

fn outcomes<'a>(
    report: &'a Replay,
    alternatives: &mut BTreeMap<MatchRule, HashMap<Key<'a>, usize>>,
) -> Result<Vec<Bucket<'a>>, String> {
    let rule = report.match_rule.normalized();
    let matcher = Matcher::new(&rule, &report.recorded)?;
    let recorded = Key::Scored {
        kind: &report.recorded.kind,
        label: &report.recorded.label,
        projection: matcher
            .project(&report.recorded)
            .ok_or("replay rule cannot project the recorded outcome")?,
    };
    let conditions = std::iter::once(&report.baseline)
        .chain(report.variants.iter().map(|variant| &variant.trials))
        .chain(report.pair.iter().map(|pair| &pair.trials));
    let mut groups = HashMap::new();
    for (condition, trials) in conditions.enumerate() {
        for (trial, result) in trials.iter().enumerate() {
            let key = match result {
                Trial::Completed { choice } => match matcher.project(choice) {
                    Some(projection) => Key::Scored {
                        kind: &choice.kind,
                        label: &choice.label,
                        projection,
                    },
                    None => Key::Unscored,
                },
                Trial::Failed => Key::Failed,
            };
            let group = groups.entry(key).or_insert_with(|| Occurrences {
                example: TrialPosition { condition, trial },
                counts: Vec::new(),
            });
            match group.counts.last_mut() {
                Some(entry) if entry.condition == condition => entry.count += 1,
                _ => group.counts.push(ConditionCount {
                    condition,
                    count: 1,
                }),
            }
        }
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_unstable_by_key(|(_, group)| (group.example.condition, group.example.trial));
    let ids = alternatives.entry(rule.clone()).or_default();
    let buckets = groups
        .into_iter()
        .map(|(key, Occurrences { example, counts })| {
            let status = match &key {
                Key::Scored { .. } if key == recorded => Status::Match,
                Key::Scored { .. } => Status::Alternative,
                Key::Unscored => Status::Unscored,
                Key::Failed => Status::Failed,
            };
            let selection = match (&key, &rule) {
                (
                    Key::Scored {
                        projection: Projection::Fields(values),
                        ..
                    },
                    MatchRule::OutcomeFields { pointers },
                ) => Some(Selection::Fields(
                    pointers
                        .iter()
                        .cloned()
                        .zip(values.iter().cloned())
                        .collect(),
                )),
                (
                    Key::Scored {
                        projection: Projection::Capture(value),
                        ..
                    },
                    _,
                ) => Some(Selection::Capture(value)),
                _ => None,
            };
            let alternative_id = (status == Status::Alternative).then(|| {
                let id = ids.len();
                *ids.entry(key).or_insert(id)
            });
            Bucket {
                status,
                example,
                alternative_id,
                selection,
                counts,
            }
        })
        .collect();
    Ok(buckets)
}
