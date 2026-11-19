//! Recurring links between captured context and decisions.

use crate::{
    context_links::{ContextLink, LinkKind},
    Basis, ContextItem, Run,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct ContextPattern<'a> {
    #[serde(flatten)]
    pub key: PatternKey<'a>,
    pub occurrences: Vec<ContextOccurrence<'a>>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PatternKey<'a> {
    pub trace_source: &'a str,
    pub decision_kind: &'a str,
    pub decision_label: &'a str,
    pub source: ContextSource<'a>,
    pub relationship: ContextRelationship<'a>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextSource<'a> {
    Named {
        id: &'a str,
        version: Option<&'a str>,
        text_sha256: Option<String>,
    },
    ToolDefinition {
        json_sha256: String,
    },
    ToolResult {
        tool: &'a str,
    },
    UserInput,
    CapturedText {
        context_kind: &'a str,
        role: Option<&'a str>,
        text_sha256: String,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextRelationship<'a> {
    Annotation {
        basis: Basis,
        relation: &'a str,
    },
    ToolDefinition,
    ArgumentMatch {
        argument_path: Option<String>,
        context_path: Option<String>,
    },
    OutcomeMatch {
        outcome_path: Option<String>,
        context_path: Option<String>,
    },
    OutcomeIdentifier {
        context_path: Option<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct ContextOccurrence<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub decision_id: &'a str,
    pub context_id: &'a str,
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub evidence: LinkEvidence,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkEvidence {
    ExactMatch { index: usize },
    Annotation { index: usize },
}

pub(crate) fn group<'a>(
    runs: impl Iterator<Item = &'a Run>,
    links: &[ContextLink<'a>],
) -> Vec<ContextPattern<'a>> {
    let mut links_by_call: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (index, link) in links.iter().enumerate() {
        links_by_call
            .entry((link.run_id, link.call_id))
            .or_default()
            .push((index, link));
    }

    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for run in runs {
        for call in &run.calls {
            let links = links_by_call
                .get(&(run.id.as_str(), call.id.as_str()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if links.is_empty() && call.relationships.is_empty() {
                continue;
            }
            let items: BTreeMap<_, _> = call
                .context
                .iter()
                .map(|item| (item.id.as_str(), item))
                .collect();
            let decisions: BTreeMap<_, _> = call
                .decisions
                .iter()
                .map(|decision| (decision.id.as_str(), decision))
                .collect();
            let mut record =
                |decision_id, item: &'a ContextItem, relationship, source, evidence| {
                    let decision = decisions[decision_id];
                    let key = PatternKey {
                        trace_source: &run.source,
                        decision_kind: &decision.kind,
                        decision_label: &decision.label,
                        source,
                        relationship,
                    };
                    groups.entry(key).or_default().push(ContextOccurrence {
                        run_id: &run.id,
                        call_id: &call.id,
                        decision_id,
                        context_id: &item.id,
                        organization_id: run.organization_id.as_deref(),
                        user_id: run.user_id.as_deref(),
                        conversation_id: run.conversation_id.as_deref(),
                        turn_id: call.turn_id.as_deref(),
                        started_at: call.started_at.as_deref(),
                        evidence,
                    });
                };

            for &(index, link) in links {
                let item = items[link.context_id];
                let Some(source) = context_source(item, Some(link)) else {
                    continue;
                };
                let relationship = match link.kind {
                    LinkKind::ToolDefinition => ContextRelationship::ToolDefinition,
                    LinkKind::ArgumentMatch => ContextRelationship::ArgumentMatch {
                        argument_path: link.argument_path.clone(),
                        context_path: link.context_path.clone(),
                    },
                    LinkKind::OutcomeMatch => ContextRelationship::OutcomeMatch {
                        outcome_path: link.outcome_path.clone(),
                        context_path: link.context_path.clone(),
                    },
                    LinkKind::OutcomeIdentifier => ContextRelationship::OutcomeIdentifier {
                        context_path: link.context_path.clone(),
                    },
                };
                record(
                    link.decision_id,
                    item,
                    relationship,
                    source,
                    LinkEvidence::ExactMatch { index },
                );
            }
            for (index, annotation) in call.relationships.iter().enumerate() {
                let item = items[annotation.context_id.as_str()];
                let Some(source) = context_source(item, None) else {
                    continue;
                };
                let relationship = ContextRelationship::Annotation {
                    basis: annotation.basis,
                    relation: &annotation.relation,
                };
                record(
                    annotation.decision_id.as_str(),
                    item,
                    relationship,
                    source,
                    LinkEvidence::Annotation { index },
                );
            }
        }
    }
    groups
        .into_iter()
        .map(|(key, occurrences)| ContextPattern { key, occurrences })
        .collect()
}

fn context_source<'a>(
    item: &'a ContextItem,
    link: Option<&ContextLink<'a>>,
) -> Option<ContextSource<'a>> {
    if let Some(id) = item.source_id.as_deref() {
        let version = item.source_version.as_deref();
        return Some(ContextSource::Named {
            id,
            version,
            text_sha256: if version.is_none() {
                item.text.as_deref().map(fingerprint)
            } else {
                None
            },
        });
    }
    if let Some(link) = link {
        if link.kind == LinkKind::ToolDefinition {
            let text = item.text.as_deref()?;
            let definition = if link.context_path.is_none() {
                serde_json::Value::String(text.to_owned())
            } else {
                serde_json::from_str(text).ok()?
            };
            return Some(ContextSource::ToolDefinition {
                json_sha256: fingerprint(&crate::canonical_json(&definition)),
            });
        }
        if let Some(origin) = link.source_tool {
            return Some(ContextSource::ToolResult { tool: origin.tool });
        }
        if item.role.as_deref() == Some("user")
            && matches!(item.kind.as_str(), "message" | "message_part" | "text")
        {
            return Some(ContextSource::UserInput);
        }
    }
    Some(ContextSource::CapturedText {
        context_kind: &item.kind,
        role: item.role.as_deref(),
        text_sha256: fingerprint(item.text.as_deref()?),
    })
}

pub(crate) fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
