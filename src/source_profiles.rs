//! Named context exposure across traces, including context without decision links.

pub mod query;

use crate::{context_links::ContextLink, context_patterns::fingerprint, Availability, Run};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct SourceProfile<'a> {
    pub source_id: &'a str,
    pub occurrences: Vec<SourceOccurrence<'a>>,
}

#[derive(Debug, Serialize)]
pub struct SourceOccurrence<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub context_id: &'a str,
    pub trace_source: &'a str,
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub availability: Availability,
    pub text_sha256: Option<String>,
    pub linked_decision_ids: Vec<&'a str>,
}

pub(crate) fn group<'a>(
    runs: impl Iterator<Item = &'a Run>,
    links: &[ContextLink<'a>],
) -> Vec<SourceProfile<'a>> {
    let mut linked = BTreeMap::<_, BTreeSet<_>>::new();
    for link in links {
        linked
            .entry((link.run_id, link.call_id, link.context_id))
            .or_default()
            .insert(link.decision_id);
    }

    let mut profiles = BTreeMap::<_, Vec<_>>::new();
    for run in runs {
        for call in &run.calls {
            for annotation in &call.relationships {
                linked
                    .entry((
                        run.id.as_str(),
                        call.id.as_str(),
                        annotation.context_id.as_str(),
                    ))
                    .or_default()
                    .insert(annotation.decision_id.as_str());
            }
            for item in &call.context {
                let Some(source_id) = item.source_id.as_deref().filter(|id| !id.trim().is_empty())
                else {
                    continue;
                };
                let key = (run.id.as_str(), call.id.as_str(), item.id.as_str());
                profiles
                    .entry(source_id)
                    .or_default()
                    .push(SourceOccurrence {
                        run_id: &run.id,
                        call_id: &call.id,
                        context_id: &item.id,
                        trace_source: &run.source,
                        organization_id: run.organization_id.as_deref(),
                        user_id: run.user_id.as_deref(),
                        conversation_id: run.conversation_id.as_deref(),
                        turn_id: call.turn_id.as_deref(),
                        started_at: call.started_at.as_deref(),
                        source_version: item.source_version.as_deref(),
                        availability: item.availability,
                        text_sha256: item.text.as_deref().map(fingerprint),
                        linked_decision_ids: linked
                            .remove(&key)
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    });
            }
        }
    }
    profiles
        .into_iter()
        .map(|(source_id, occurrences)| SourceProfile {
            source_id,
            occurrences,
        })
        .collect()
}
