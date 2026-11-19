//! Group comparable interventions and retain the individual decision evidence.

use crate::{replay, Call, Run};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct EffectGroup<'a> {
    #[serde(flatten)]
    pub comparison: Comparison<'a>,
    pub occurrences: Vec<EffectOccurrence<'a>>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Comparison<'a> {
    #[serde(flatten)]
    pub target: ComparisonTarget<'a>,
    pub runner_id: &'a str,
    #[serde(skip_serializing_if = "replay::MatchRule::is_exact")]
    pub match_rule: replay::MatchRule,
    #[serde(skip_serializing_if = "replay::SamplingDesign::is_unspecified")]
    pub sampling_design: replay::SamplingDesign,
    pub model: Option<&'a str>,
    pub decision_kind: &'a str,
    pub decision_label: &'a str,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(untagged)]
pub enum ComparisonTarget<'a> {
    Single(ContextChange<'a>),
    Pair {
        pair: [ContextChange<'a>; 2],
    },
    Control {
        target_edit: ContextChange<'a>,
        control_edit: ContextChange<'a>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ContextChange<'a> {
    pub source_id: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub context_sha256: &'a str,
    pub context_kind: &'a str,
    pub context_role: Option<&'a str>,
    #[serde(skip_serializing_if = "replay::Operation::is_text")]
    pub operation: replay::Operation,
    pub replacement: &'a str,
}

#[derive(Debug, Serialize)]
pub struct EffectOccurrence<'a> {
    pub location: &'a replay::Location,
    pub replay_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rescored_from: Option<&'a str>,
    #[serde(flatten)]
    pub target: OccurrenceTarget<'a>,
    pub trace_source: &'a str,
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub baseline: &'a replay::Summary,
    pub variant: &'a replay::Summary,
    pub difference_pp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difference_95_interval_pp: Option<replay::ReplayInterval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_95_interval_pp: Option<replay::ReplayInterval>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OccurrenceTarget<'a> {
    Single {
        context_id: &'a str,
    },
    Pair {
        context_ids: [&'a str; 2],
        interaction_pp: Option<f64>,
    },
    Control {
        target_context_id: &'a str,
        control_context_id: &'a str,
        control: &'a replay::Summary,
    },
}

#[derive(Debug, Serialize)]
pub struct ContextEffects<'a> {
    #[serde(flatten)]
    pub location: &'a replay::Location,
    pub context_id: &'a str,
    pub effects: Vec<EffectReference>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct EffectReference {
    pub group_index: usize,
    pub occurrence_index: usize,
}

/// Map replay evidence to exact context instances in the same recorded decision.
pub fn by_context<'a>(groups: &[EffectGroup<'a>]) -> Vec<ContextEffects<'a>> {
    let mut index: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (group_index, group) in groups.iter().enumerate() {
        for (occurrence_index, occurrence) in group.occurrences.iter().enumerate() {
            let contexts = match &occurrence.target {
                OccurrenceTarget::Single { context_id } => std::slice::from_ref(context_id),
                OccurrenceTarget::Pair { context_ids, .. } => context_ids.as_slice(),
                // The contrast belongs to the design. Each edit already has its own effect.
                OccurrenceTarget::Control { .. } => continue,
            };
            for &context_id in contexts {
                index
                    .entry((occurrence.location, context_id))
                    .or_default()
                    .push(EffectReference {
                        group_index,
                        occurrence_index,
                    });
            }
        }
    }
    index
        .into_iter()
        .map(|((location, context_id), effects)| ContextEffects {
            location,
            context_id,
            effects,
        })
        .collect()
}

/// Index reports after validating them against the evidence returned by `inspect`.
pub fn group<'a>(
    runs: &[&'a Run],
    reports: &'a [replay::Replay],
) -> Result<Vec<EffectGroup<'a>>, String> {
    let runs: BTreeMap<_, _> = runs.iter().map(|&run| (run.id.as_str(), run)).collect();
    let mut comparisons: BTreeMap<_, Vec<_>> = BTreeMap::new();

    for report in reports {
        let location = &report.location;
        let run = runs
            .get(location.run_id.as_str())
            .ok_or("replay run is missing")?;
        let call = run
            .calls
            .iter()
            .find(|call| call.id == location.call_id)
            .ok_or("replay call is missing")?;
        let match_rule = report.match_rule.normalized();
        let mut record =
            |comparison_target,
             target,
             variant,
             difference_pp,
             (difference_95_interval_pp, interaction_95_interval_pp)| {
                let comparison = Comparison {
                    target: comparison_target,
                    runner_id: &report.runner_id,
                    match_rule: match_rule.clone(),
                    sampling_design: report.sampling_design,
                    model: call.model.as_deref(),
                    decision_kind: &report.recorded.kind,
                    decision_label: &report.recorded.label,
                };
                comparisons
                    .entry(comparison)
                    .or_default()
                    .push(EffectOccurrence {
                        location,
                        replay_id: &report.id,
                        rescored_from: report.rescored_from.as_deref(),
                        target,
                        trace_source: &run.source,
                        organization_id: run.organization_id.as_deref(),
                        user_id: run.user_id.as_deref(),
                        conversation_id: run.conversation_id.as_deref(),
                        turn_id: call.turn_id.as_deref(),
                        started_at: call.started_at.as_deref(),
                        baseline: &report.baseline_summary,
                        variant,
                        difference_pp,
                        difference_95_interval_pp,
                        interaction_95_interval_pp,
                    });
            };
        let changes = report
            .variants
            .iter()
            .map(|variant| context_change(call, variant))
            .collect::<Result<Vec<_>, _>>()?;
        for (&change, variant) in changes.iter().zip(&report.variants) {
            record(
                ComparisonTarget::Single(change),
                OccurrenceTarget::Single {
                    context_id: &variant.intervention.context_id,
                },
                &variant.summary,
                variant.difference_pp,
                (variant.difference_95_interval_pp, None),
            );
        }
        if let Some(pair) = &report.pair {
            let [left, right] = changes[..] else {
                return Err("a pair report requires exactly two single interventions".into());
            };
            let mut targets = [
                (left, report.variants[0].intervention.context_id.as_str()),
                (right, report.variants[1].intervention.context_id.as_str()),
            ];
            targets.sort_unstable();
            record(
                ComparisonTarget::Pair {
                    pair: targets.map(|(change, _)| change),
                },
                OccurrenceTarget::Pair {
                    context_ids: targets.map(|(_, id)| id),
                    interaction_pp: pair.interaction_pp,
                },
                &pair.summary,
                pair.difference_pp,
                (
                    pair.difference_95_interval_pp,
                    pair.interaction_95_interval_pp,
                ),
            );
        }
        if let Some(comparison) = &report.control_comparison {
            let [first, second] = &report.variants[..] else {
                return Err("a controlled comparison requires two edits".into());
            };
            let (target, control) =
                if first.intervention.context_id == comparison.control_context_id {
                    (1, 0)
                } else if second.intervention.context_id == comparison.control_context_id {
                    (0, 1)
                } else {
                    return Err("controlled comparison is missing its control edit".into());
                };
            record(
                ComparisonTarget::Control {
                    target_edit: changes[target],
                    control_edit: changes[control],
                },
                OccurrenceTarget::Control {
                    target_context_id: &report.variants[target].intervention.context_id,
                    control_context_id: &report.variants[control].intervention.context_id,
                    control: &report.variants[control].summary,
                },
                &report.variants[target].summary,
                comparison.target_minus_control_pp,
                (comparison.target_minus_control_95_interval_pp, None),
            );
        }
    }
    Ok(comparisons
        .into_iter()
        .map(|(comparison, occurrences)| EffectGroup {
            comparison,
            occurrences,
        })
        .collect())
}

fn context_change<'a>(
    call: &'a Call,
    variant: &'a replay::Variant,
) -> Result<ContextChange<'a>, String> {
    let intervention = &variant.intervention;
    let context = call
        .context
        .iter()
        .find(|item| item.id == intervention.context_id)
        .ok_or("replay context is missing")?;
    Ok(ContextChange {
        source_id: context.source_id.as_deref(),
        source_version: context.source_version.as_deref(),
        context_sha256: &variant.captured_context_sha256,
        context_kind: &context.kind,
        context_role: context.role.as_deref(),
        operation: intervention.operation,
        replacement: &intervention.replacement,
    })
}
