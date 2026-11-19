use crate::{
    activation::{
        decision_path, decision_view_path,
        visibility::query::{measured_masks, SourceMask},
        Report,
    },
    conflict_query::{visit_rule_checks, Phase},
    replay::{
        self, candidates,
        probe_patterns::{source_path, Kind},
        shadow,
    },
    turn_evidence::inspector_path,
    Call, ContextItem, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Input<'a> = (&'a str, &'a str, &'a str);

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Filter {
    #[default]
    All,
    Conflicts,
    Measured,
    Unmeasured,
}

impl Filter {
    pub(super) fn includes(self, evidence: Option<&InputEvidence<'_>>) -> bool {
        match self {
            Self::All => true,
            Self::Conflicts => evidence.is_some_and(InputEvidence::conflicted),
            Self::Measured => evidence.is_some_and(InputEvidence::measured),
            Self::Unmeasured => !evidence.is_some_and(InputEvidence::measured),
        }
    }
}

#[derive(Default)]
pub(super) struct InputEvidence<'a> {
    pub replay_count: usize,
    pub mask_count: usize,
    pub candidate_count: usize,
    pub shadow_count: usize,
    pub shadow_unavailable: usize,
    pub decision_conflicts: usize,
    replay: Option<(&'a replay::Replay, &'a replay::Variant)>,
    mask: Option<SourceMask<'a>>,
    candidate: Option<(&'a str, f64)>,
    shadow: Option<(&'a str, Option<f64>)>,
    preflight: Option<Conflict<'a>>,
    decision: Option<Conflict<'a>>,
}

struct Conflict<'a> {
    scope: (&'a str, &'a str),
    opposing: &'a ContextItem,
    decision_id: Option<&'a str>,
}

impl Conflict<'_> {
    fn reference(&self, (run, call, input): Input<'_>) -> Value {
        let (key, scope) = self.scope;
        let inspector_path = if let Some(decision_id) = self.decision_id {
            let path = decision_view_path(run, call, Some(decision_id));
            let mut ids = [input, self.opposing.id.as_str()];
            // The browser sorts finding IDs by UTF-16 code units.
            ids.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            let finding = json!(["conflict", key, scope, ids[0], ids[1]]).to_string();
            format!("{path}&finding={}", encode(&finding))
        } else {
            let path = inspector_path(run, call, None);
            format!("{path}&preflight_source={}", encode(input))
        };
        json!({
            "key": key, "scope": scope, "decision_id": self.decision_id,
            "opposing_context_id": self.opposing.id, "opposing_source_id": self.opposing.source_id,
            "opposing_source_version": self.opposing.source_version, "inspector_path": inspector_path,
        })
    }
}

impl InputEvidence<'_> {
    pub(super) fn measured(&self) -> bool {
        self.replay_count > 0
            || self.mask_count > 0
            || self.candidate_count > 0
            || self.shadow_count > 0
    }

    pub(super) fn preflight_conflict(&self) -> bool {
        self.preflight.is_some()
    }

    pub(super) fn conflicted(&self) -> bool {
        self.preflight_conflict() || self.decision_conflicts > 0
    }

    pub(super) fn describe(&self, key @ (run, call, context): Input<'_>, value: &mut Value) {
        value["replay_measurement_count"] = json!(self.replay_count);
        value["mask_measurement_count"] = json!(self.mask_count);
        value["candidate_measurement_count"] = json!(self.candidate_count);
        value["shadow_measurement_count"] = json!(self.shadow_count);
        value["shadow_unavailable_count"] = json!(self.shadow_unavailable);
        value["preflight_conflict"] = json!(self.preflight_conflict());
        value["decision_conflict_count"] = json!(self.decision_conflicts);
        if let Some((report, variant)) = self.replay {
            let path = decision_path(&report.location);
            let replay = encode(&report.id);
            let context = encode(context);
            value["replay_example"] = json!({
                "replay_id": report.id, "location": report.location, "difference_pp": variant.difference_pp,
                "inspector_path": format!("{path}&replay={replay}&variant={context}"),
            });
        }
        if let Some(mask) = &self.mask {
            value["mask_example"] = mask.reference();
        }
        if let Some((report, shift)) = self.candidate {
            value["candidate_example"] = json!({
                "report_id": report, "change_log_odds": shift,
                "inspector_path": source_path(Kind::Candidate, run, call, report, context),
            });
        }
        if let Some((report, shift)) = self.shadow {
            value["shadow_example"] = json!({
                "report_id": report, "difference_pp": shift,
                "inspector_path": source_path(Kind::Shadow, run, call, report, context),
            });
        }
        if let Some(conflict) = &self.preflight {
            value["preflight_conflict_example"] = conflict.reference(key);
        }
        if let Some(conflict) = &self.decision {
            value["decision_conflict_example"] = conflict.reference(key);
        }
    }
}

pub(super) fn index<'a>(
    eligible: &BTreeSet<Input<'a>>,
    runs: &[&'a Run],
    replays: &'a [replay::Replay],
    reports: &'a [Report],
) -> BTreeMap<Input<'a>, InputEvidence<'a>> {
    let mut index = BTreeMap::<_, InputEvidence<'_>>::new();
    for report in replays {
        for variant in &report.variants {
            let key = (
                report.location.run_id.as_str(),
                report.location.call_id.as_str(),
                variant.intervention.context_id.as_str(),
            );
            if variant.difference_pp.is_some() && eligible.contains(&key) {
                let entry = index.entry(key).or_default();
                entry.replay_count += 1;
                entry.replay.get_or_insert((report, variant));
            }
        }
    }
    for mask in measured_masks(reports) {
        let (location, id) = mask.input();
        let key = (location.run_id.as_str(), location.call_id.as_str(), id);
        if eligible.contains(&key) {
            let entry = index.entry(key).or_default();
            entry.mask_count += 1;
            entry.mask.get_or_insert(mask);
        }
    }
    let calls: BTreeSet<_> = eligible.iter().map(|&(run, call, _)| (run, call)).collect();
    for run in runs {
        for call in &run.calls {
            if calls.contains(&(run.id.as_str(), call.id.as_str())) {
                index_probes(&run.id, call, eligible, &mut index);
                index_conflicts(&run.id, call, eligible, &mut index);
            }
        }
    }
    index
}

fn index_probes<'a>(
    run: &'a str,
    call: &'a Call,
    eligible: &BTreeSet<Input<'_>>,
    index: &mut BTreeMap<Input<'a>, InputEvidence<'a>>,
) {
    for probe in candidates::recorded(run, call) {
        let candidates::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        let report = probe.report_id.expect("verified candidate probe");
        for effect in measurement.effects {
            let key = (run, call.id.as_str(), effect.context_id);
            if eligible.contains(&key) {
                let entry = index.entry(key).or_default();
                entry.candidate_count += 1;
                entry
                    .candidate
                    .get_or_insert((report, effect.change_log_odds));
            }
        }
    }
    for probe in shadow::recorded(run, call) {
        let shadow::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        let report = probe.report_id.expect("verified shadow probe");
        for effect in measurement.effects {
            let key = (run, call.id.as_str(), effect.context_id);
            if eligible.contains(&key) {
                let entry = index.entry(key).or_default();
                let measured = effect.difference_pp.is_some();
                entry.shadow_count += usize::from(measured);
                entry.shadow_unavailable += usize::from(!measured);
                if entry.shadow.is_none() || (measured && entry.shadow_count == 1) {
                    entry.shadow = Some((report, effect.difference_pp));
                }
            }
        }
    }
}

fn index_conflicts<'a>(
    run: &'a str,
    call: &'a Call,
    eligible: &BTreeSet<Input<'_>>,
    index: &mut BTreeMap<Input<'a>, InputEvidence<'a>>,
) {
    for phase in [Phase::Preflight, Phase::Decision] {
        visit_rule_checks(call, phase, None, |check| {
            let Some(sources) = check.comparison().1 else {
                return;
            };
            for (item, rule) in check.rules {
                let key = (run, call.id.as_str(), item.id.as_str());
                let Some(value) = rule.value.as_ref().filter(|_| eligible.contains(&key)) else {
                    continue;
                };
                let decision_id = check.decision.map(|decision| decision.id.as_str());
                let conflict = Conflict {
                    scope: check.scope,
                    opposing: sources.opposing_source(value),
                    decision_id,
                };
                let entry = index.entry(key).or_default();
                if decision_id.is_some() {
                    entry.decision_conflicts += 1;
                    entry.decision.get_or_insert(conflict);
                } else {
                    entry.preflight.get_or_insert(conflict);
                }
            }
        });
    }
}

pub(super) fn encode(value: &str) -> impl std::fmt::Display + '_ {
    utf8_percent_encode(value, NON_ALPHANUMERIC)
}
