//! References to measured decoder-block swaps at captured source tokens.

use super::{activation_path, Report, Sweep, Target};
use crate::replay::Location;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum Position<'a> {
    Point(usize),
    Checkpoint {
        index: usize,
        output_token: usize,
        label: &'a str,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct SourceSwap<'a> {
    pub location: &'a Location,
    pub context_id: &'a str,
    report_id: &'a str,
    position: Position<'a>,
    block: usize,
    baseline: f64,
    patched: f64,
}

impl SourceSwap<'_> {
    pub(crate) fn change(self) -> f64 {
        self.patched - self.baseline
    }

    pub(crate) fn reference(self) -> Value {
        let (key, index, position) = match self.position {
            Position::Point(index) => ("point", index, json!({"prediction_position": index})),
            Position::Checkpoint {
                index,
                output_token,
                label,
            } => (
                "checkpoint",
                index,
                json!({"checkpoint_index": index, "output_token": output_token, "label": label}),
            ),
        };
        json!({
            "report_id": self.report_id, "location": self.location, "position": position,
            "block": self.block, "original_log_odds": self.baseline, "patched_log_odds": self.patched,
            "change_log_odds": self.change(),
            "inspector_path": inspector_path(self.location, self.report_id, key, index, self.block, self.context_id),
        })
    }
}

pub(crate) fn inspector_path(
    location: &Location,
    report: &str,
    position: &str,
    index: usize,
    block: usize,
    context: &str,
) -> String {
    let path = activation_path(location, report, position, index, block);
    let source = utf8_percent_encode(context, NON_ALPHANUMERIC);
    format!("{path}&source_swap={source}")
}

/// Yield one observation per source sweep and measured prediction position.
/// Keep the largest absolute shift at any block in that sweep.
pub(crate) fn measured(reports: &[Report]) -> impl Iterator<Item = SourceSwap<'_>> {
    reports.iter().flat_map(|report| {
        let positions: Vec<(Position<'_>, f64, &[Sweep])> = match report {
            Report::Point(report) => vec![(
                Position::Point(report.prediction_position),
                report.baseline.margin,
                &report.sweeps,
            )],
            Report::Timeline(report) => report
                .checkpoints
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    (
                        Position::Checkpoint {
                            index,
                            output_token: step.output_token,
                            label: &step.label,
                        },
                        step.baseline_margin,
                        step.sweeps.as_slice(),
                    )
                })
                .collect(),
            Report::Visibility(_) => Vec::new(),
        };
        positions
            .into_iter()
            .flat_map(move |(position, baseline, sweeps)| {
                sweeps
                    .iter()
                    .filter(|sweep| sweep.target == Target::SourceTokens)
                    .filter_map(move |sweep| {
                        let (block, &patched) =
                            sweep
                                .margins
                                .iter()
                                .enumerate()
                                .rev()
                                .max_by(|(_, a), (_, b)| {
                                    (*a - baseline).abs().total_cmp(&(*b - baseline).abs())
                                })?;
                        Some(SourceSwap {
                            location: report.location(),
                            context_id: &sweep.source_context_id,
                            report_id: report.id(),
                            position,
                            block,
                            baseline,
                            patched,
                        })
                    })
            })
    })
}
