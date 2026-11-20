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

pub(super) fn digest(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value))
}

/// A factor links evidence about the same source set. Each experiment retains its own comparison.
pub(super) fn factor_id(run: &Run, observation: &Observation) -> String {
    let sources = observation.detail["sources"]
        .as_array()
        .expect("finding sources");
    // The repeat preview shows only the first two sources, so preserve its full group identity.
    let full_repeat = (observation.detail["copies"].as_u64().unwrap_or(0) > 2)
        .then_some(observation.group.as_str());
    source_set_id(run, sources, full_repeat)
}

pub(super) fn source_set_id(run: &Run, sources: &[Value], full_repeat: Option<&str>) -> String {
    let mut identities: Vec<_> = sources
        .iter()
        .map(|source| {
            let stable = source["source"].as_str();
            (
                stable.is_none().then_some(run.id.as_str()),
                stable.or(source["id"].as_str()),
                source["version"].as_str(),
            )
        })
        .collect();
    identities.sort_unstable();
    digest(json!([run.source, run.organization_id, identities, full_repeat]).to_string())
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
    let action = super::recommendations::action(kind, &detail);
    Observation {
        kind,
        group: digest(key.to_string()),
        call_id: call.id.clone(),
        observed: call.started_at.clone(),
        detail,
        action,
    }
}

pub(super) fn source_gap(
    run: &Run,
    call: &Call,
    coverage: &nexusweave::preflight::SourceCoverage,
) -> Option<Observation> {
    if coverage.matched {
        return None;
    }
    // Alternative order has no effect on whether the requirement is met.
    let mut routes = BTreeSet::new();
    let mut expected = BTreeSet::new();
    for alternative in &coverage.alternatives {
        let required: BTreeSet<_> = alternative.required.iter().collect();
        expected.extend(required.iter().copied());
        routes.insert((
            required,
            alternative.unconfirmed.iter().collect::<BTreeSet<_>>(),
        ));
    }
    let present: Vec<_> = call
        .context
        .iter()
        .filter(|item| {
            item.availability == Availability::Included
                && item.request_fragment.is_some()
                && item
                    .source_id
                    .as_ref()
                    .is_some_and(|id| expected.contains(id))
        })
        .collect();
    let versions: BTreeSet<_> = present
        .iter()
        .map(|item| (&item.source_id, &item.source_version))
        .collect();
    let mut key = vec![
        json!("source_gap"),
        json!(run.source),
        json!(run.organization_id),
        json!(coverage.name),
        json!(routes),
        json!(versions),
    ];
    if !coverage.expected_text_sha256.is_empty() {
        key.push(json!(coverage
            .expected_text_sha256
            .keys()
            .collect::<Vec<_>>()));
        key.push(json!(coverage.text_mismatches));
    }
    let sources: Vec<_> = present
        .into_iter()
        .map(|item| {
            let mut evidence = source(item);
            if item
                .source_id
                .as_ref()
                .is_some_and(|id| coverage.text_mismatches.contains(id))
            {
                evidence["request_fragment"] = json!(item.request_fragment);
                evidence["captured_text_sha256"] = json!(item.text.as_ref().map(digest));
            }
            evidence
        })
        .collect();
    Some(occurrence(
        "source_gap",
        Value::Array(key),
        call,
        json!({"coverage":coverage,"sources":sources}),
    ))
}

pub fn derive(
    run: &Run,
    reports: &[replay::Replay],
    activations: &[activation::Report],
) -> Result<Vec<Observation>, String> {
    let mut findings = Vec::new();
    for call in &run.calls {
        if let Some(coverage) = nexusweave::preflight::verified_source_coverage(call) {
            findings.extend(source_gap(run, call, &coverage));
        }
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
            let dialogue_only = entries.iter().all(|item| {
                item.source_id.is_none()
                    && matches!(item.role.as_deref(), Some("user" | "assistant"))
                    && matches!(item.kind.as_str(), "text" | "message")
                    && item.evidence.message_index.is_some()
            });
            findings.push(occurrence("duplicate", json!(["duplicate", digest(text), sources]), call,
                json!({"sources": [source(entries[0]), source(entries[1])], "copies": entries.len(), "dialogue_only": dialogue_only})));
        }
    }
    let structural_count = findings.len();
    for call in &run.calls {
        super::recommendations::probes(run, call, reports, &mut findings);
    }
    super::recommendations::token_measurements(run, activations, &mut findings);
    super::recommendations::tool_mask_measurements(run, activations, &mut findings);
    let by_location: BTreeMap<_, _> = reports
        .iter()
        .map(|report| ((&report.location, report.id.as_str()), report))
        .collect();
    let mut layouts = BTreeMap::new();
    let mut captured_inputs = BTreeMap::new();
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
            let ids = match &item.target {
                replay_effects::OccurrenceTarget::Single { context_id } => vec![*context_id],
                replay_effects::OccurrenceTarget::Pair { context_ids, .. } => context_ids.to_vec(),
                replay_effects::OccurrenceTarget::Control {
                    target_context_id,
                    control_context_id,
                    ..
                } => vec![*target_context_id, *control_context_id],
            };
            let sources: Vec<_> = sources.iter().zip(ids)
                .map(|(change,id)| json!({"id": id,"source": change.source_id, "version": change.source_version, "role": change.context_role}))
                .collect();
            let report = by_location[&(item.location, item.replay_id)];
            let single_edits: Vec<_> = report.variants.iter().filter(|_| edit == "joint").map(|arm| {
                let proposal = super::outcomes::consistent(&arm.trials);
                json!({"id":arm.intervention.context_id,"replacement":arm.intervention.replacement.chars().take(200).collect::<String>(),
                    "summary":arm.summary,"difference_pp":arm.difference_pp,
                    "changed":proposal.zip(super::outcomes::consistent(&report.baseline)).map(|(edited,base)| edited!=base),
                    "proposal":proposal.map(super::outcomes::preview)})
            }).collect();
            let joint_only = edit == "joint"
                && effect != 0.0
                && report
                    .variants
                    .iter()
                    .all(|arm| arm.difference_pp == Some(0.0));
            let mut detail = json!({"effect_pp": effect, "measurement": item, "sources": sources, "edit": edit,
                       "runner": group.comparison.runner_id, "model": group.comparison.model,
                       "single_edits":single_edits, "joint_only":joint_only, "outcome": group.comparison.decision_label, "match_rule": group.comparison.match_rule,
                       "sampling_design": group.comparison.sampling_design});
            let input = if let Some(captured) = &call.recorded_request {
                use std::collections::btree_map::Entry;
                Some(match captured_inputs.entry(call.id.as_str()) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        entry.insert(super::variation::Input::new(call, captured)?)
                    }
                })
            } else {
                None
            };
            if edit == "joint" {
                detail["joint_input"] = json!(input.as_ref().map(|i| &i.digest));
                detail["joint_pattern"] =
                    json!(super::outcomes::joint_pattern(report).unwrap_or("unresolved"));
                if let Some(pair) = &report.pair {
                    detail["response_pair"] =
                        json!(super::outcomes::compare(&report.baseline, &pair.trials));
                }
                if let Some(trial) = super::order::trial(call, report) {
                    detail["order_test"] = trial;
                }
            }
            if let replay_effects::OccurrenceTarget::Single { context_id } = &item.target {
                if let Some(variant) = report
                    .variants
                    .iter()
                    .find(|v| v.intervention.context_id == *context_id)
                {
                    if let Some(input) = input {
                        input.annotate(&mut detail, &variant.intervention);
                    }
                    detail["response_pair"] =
                        json!(super::outcomes::compare(&report.baseline, &variant.trials));
                    detail["baseline_answer"] =
                        json!(super::replacement::consistent_answer(&report.baseline));
                    detail["edited_answer"] =
                        json!(super::replacement::consistent_answer(&variant.trials));
                }
                if run.organization_id.is_some() {
                    let layout = layouts
                        .entry(call.id.as_str())
                        .or_insert_with(|| super::history::Layout::new(call));
                    if let Some(prior) = layout
                        .as_ref()
                        .and_then(|l| l.observation(run, report, context_id))
                    {
                        detail["prior"] = prior;
                    }
                }
            }
            findings.push(occurrence(
                "replay",
                json!(["replay", group.key]),
                call,
                detail,
            ));
        }
    }
    if !reports.is_empty() {
        let (structural, measurements) = findings.split_at_mut(structural_count);
        super::conflict::annotate(run, structural, measurements);
    }
    super::replacement::compare(run, reports, &mut findings);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerate the pinned eight-session retail sample using docs/guide.md.
    #[test]
    #[ignore = "requires data/exgentic-evidence.json from the public retail sample"]
    fn public_retail_recommendations() {
        let file = std::fs::File::open("data/exgentic-evidence.json").unwrap();
        let runs: Vec<Run> = serde_json::from_reader(file).unwrap();
        let mut calls = 0;
        let mut repeats = 0;
        let mut repeated_calls = BTreeSet::new();
        let mut repeated_conversations = BTreeSet::new();
        for run in &runs {
            calls += run.calls.len();
            for observation in derive(run, &[], &[]).unwrap() {
                assert_eq!(observation.kind, "duplicate");
                assert_eq!(observation.detail["dialogue_only"], true);
                repeated_calls.insert((&run.id, observation.call_id));
                repeated_conversations.insert(&run.conversation_id);
                repeats += 1;
            }
        }
        assert_eq!(
            (
                calls,
                repeats,
                repeated_calls.len(),
                repeated_conversations.len()
            ),
            (99, 101, 83, 8)
        );
        // Reconstruct what the dashboard could show after the same number of calls per session.
        let mut store = super::super::Store::open(std::path::Path::new(":memory:")).unwrap();
        for (depth, expected) in [(1, 0), (2, 0), (4, 8), (usize::MAX, 8)] {
            let mut prefix = runs.clone();
            for run in &mut prefix {
                run.calls.truncate(depth);
            }
            store.seed(&prefix, &[], &[], &[], 1).unwrap();
            let scope = BTreeMap::new();
            let impact = store.list("impact", &scope).unwrap();
            let gaps = store
                .list("conversations-without-findings", &scope)
                .unwrap();
            let findings = impact["conversations"].as_u64().unwrap();
            assert_eq!(findings, expected);
            assert_eq!(impact["captured_conversations"], 8);
            assert_eq!(impact["without_findings"], 8 - findings);
            assert_eq!(gaps["rows"].as_array().unwrap().len() as u64, 8 - findings);
            assert_eq!(impact["proposal_outcomes"]["no_pair"], findings);
            eprintln!("{} captured calls, {findings}/8 conversations with findings, 0/8 with comparable proposals",
                prefix.iter().map(|r| r.calls.len()).sum::<usize>());
        }
    }
}
