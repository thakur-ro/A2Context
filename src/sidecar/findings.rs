use nexusweave::{activation, replay, replay_effects, Availability, Call, ContextItem, Run};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
pub struct Observation {
    pub kind: &'static str,
    pub group: String,
    pub call_id: String,
    pub observed: Option<String>,
    pub detail: Value,
    pub action: super::recommendations::Action,
}

fn digest(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(super) fn source(item: &ContextItem) -> Value {
    json!({"id": item.id, "source": item.source_id, "version": item.source_version, "role": item.role})
}

pub(super) fn identity<'a>(
    run: &'a Run,
    item: &'a ContextItem,
) -> (Option<&'a str>, &'a str, Option<&'a str>) {
    let (namespace, source) = item
        .source_id
        .as_deref()
        .map_or((Some(run.id.as_str()), item.id.as_str()), |id| (None, id));
    (namespace, source, item.source_version.as_deref())
}

pub(super) fn occurrence(
    kind: &'static str,
    key: Value,
    call: &Call,
    detail: Value,
) -> Observation {
    Observation {
        kind,
        group: digest(key.to_string()),
        call_id: call.id.clone(),
        observed: call.started_at.clone(),
        detail,
        action: super::recommendations::action(kind),
    }
}

pub fn derive(
    run: &Run,
    reports: &[replay::Replay],
    activations: &[activation::Report],
) -> Result<Vec<Observation>, String> {
    let mut findings = Vec::new();
    for call in &run.calls {
        let mut rules = BTreeMap::<_, Vec<_>>::new();
        let mut texts = BTreeMap::<_, Vec<_>>::new();
        for item in &call.context {
            if item.availability != Availability::Included {
                continue;
            }
            if let Some((rule, value)) = item
                .rule
                .as_ref()
                .and_then(|rule| Some((rule, rule.value.as_ref()?)))
            {
                rules
                    .entry((&rule.key, &rule.scope))
                    .or_default()
                    .push((item, value));
            }
            if let Some(text) = item.text.as_deref().filter(|text| !text.trim().is_empty()) {
                texts.entry(text).or_default().push(item);
            }
        }
        for ((key, scope), mut entries) in rules {
            entries.sort_by_key(|(item, _)| (identity(run, item), item.id.as_str()));
            let (left, value) = entries[0];
            if let Some(&(right, other)) = entries.iter().find(|(_, other)| *other != value) {
                findings.push(occurrence("conflict", json!(["conflict", key, scope, identity(run, left), identity(run, right)]), call,
                    json!({"rule": key, "scope": scope, "sources": [source(left), source(right)],
                           "values": [preview(value), preview(other)], "declarations": entries.len()})));
            }
        }
        let fields = if texts.values().any(|items| {
            items.len() > 1 && items.iter().any(|item| item.request_fragment.is_some())
        }) {
            call.recorded_request
                .as_ref()
                .and_then(|body| serde_json::from_str(&body.body).ok())
                .map(|request| replay::request_field_matches(call, &request))
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let pointers: BTreeMap<_, _> = fields
            .iter()
            .filter_map(|entry| {
                let [field] = entry.fields.as_slice() else {
                    return None;
                };
                Some((entry.context_id, field.pointer.as_str()))
            })
            .collect();
        for (text, mut entries) in texts {
            if entries.len() < 2 {
                continue;
            }
            entries.sort_by_key(|item| (identity(run, item), item.id.as_str()));
            let mut locations = BTreeSet::new();
            entries.retain(|item| {
                item.request_fragment.as_ref().is_none_or(|fragment| {
                    locations.insert((
                        fragment.pointer.as_str(),
                        fragment.text_range.start_byte,
                        fragment.text_range.end_byte,
                    ))
                })
            });
            entries.retain(|item| {
                item.request_fragment.is_some()
                    || (item.kind != "message" && item.evidence.field != "gen_ai.input.messages")
                    || !pointers
                        .get(item.id.as_str())
                        .is_some_and(|pointer| locations.contains(&(*pointer, 0, text.len())))
            });
            if entries.len() < 2 {
                continue;
            }
            let sources = entries
                .iter()
                .map(|item| identity(run, item))
                .collect::<Vec<_>>();
            findings.push(occurrence("duplicate", json!(["duplicate", digest(text), sources]), call,
                json!({"sources": [source(entries[0]), source(entries[1])], "copies": entries.len()})));
        }
    }
    for call in &run.calls {
        super::recommendations::probes(run, call, &mut findings);
    }
    super::recommendations::token_measurements(run, activations, &mut findings);
    for group in replay_effects::group(&[run], reports)? {
        for item in &group.occurrences {
            let Some(effect) = item.difference_pp else {
                continue;
            };
            let call = run
                .calls
                .iter()
                .find(|call| call.id == item.location.call_id)
                .ok_or("replay call is missing")?;
            let (edit, sources) = match &group.comparison.target {
                replay_effects::ComparisonTarget::Single(change) => ("individual", vec![change]),
                replay_effects::ComparisonTarget::Pair { pair } => ("joint", pair.iter().collect()),
                replay_effects::ComparisonTarget::Control {
                    target_edit,
                    control_edit,
                } => ("control", vec![target_edit, control_edit]),
            };
            let sources: Vec<_> = sources.iter()
                .map(|change| json!({"source": change.source_id, "version": change.source_version, "role": change.context_role}))
                .collect();
            findings.push(occurrence("replay", json!(["replay", group.key]), call,
                json!({"effect_pp": effect, "measurement": item, "sources": sources, "edit": edit,
                       "runner": group.comparison.runner_id, "model": group.comparison.model,
                       "outcome": group.comparison.decision_label, "match_rule": group.comparison.match_rule,
                       "sampling_design": group.comparison.sampling_design})));
        }
    }
    Ok(findings)
}

fn preview(value: &Value) -> String {
    let text = value.to_string();
    let mut chars = text.chars();
    let mut preview = chars.by_ref().take(120).collect::<String>();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}
