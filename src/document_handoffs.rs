//! Compare captured document evidence within an individual trace.

use crate::{Availability, ContextItem, Run};
use serde::Serialize;
use std::collections::BTreeMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Serialize)]
pub struct DocumentHandoff<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub context_id: &'a str,
    pub status: HandoffStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<RetrievalOrigin<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Matched,
    Ambiguous,
    TimingUnavailable,
    ContentUnavailable,
    ContentDiffers,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RetrievalOrigin<'a> {
    pub call_id: &'a str,
    pub document_id: &'a str,
}

struct Candidate<'a> {
    origin: RetrievalOrigin<'a>,
    text: Option<&'a str>,
    started_at: Option<OffsetDateTime>,
    ended_at: Option<OffsetDateTime>,
}

pub(crate) fn link<'a>(runs: impl Iterator<Item = &'a Run>) -> Vec<DocumentHandoff<'a>> {
    let mut handoffs = Vec::new();
    for run in runs {
        let mut documents = BTreeMap::<_, Vec<_>>::new();
        for call in &run.calls {
            if let Some(retrieval) = &call.retrieval {
                let start = instant(call.started_at.as_deref());
                let end = instant(call.ended_at.as_deref())
                    .filter(|end| start.is_none_or(|start| start <= *end));
                for item in &retrieval.documents {
                    if let Some(identity) = &item.document_identity {
                        documents
                            .entry(identity.id.as_str())
                            .or_default()
                            .push(Candidate {
                                origin: RetrievalOrigin {
                                    call_id: &call.id,
                                    document_id: &item.id,
                                },
                                text: text(item),
                                started_at: start,
                                ended_at: end,
                            });
                    }
                }
            }
        }
        if documents.is_empty() {
            continue;
        }
        for call in &run.calls {
            let start = instant(call.started_at.as_deref());
            for item in &call.context {
                let Some(candidates) = item
                    .document_identity
                    .as_ref()
                    .and_then(|document| documents.get(document.id.as_str()))
                else {
                    continue;
                };
                let eligible = candidates.iter().filter(|candidate| {
                    candidate.origin.call_id != call.id
                        && candidate
                            .started_at
                            .zip(start)
                            .is_none_or(|(source, start)| source <= start)
                        && candidate
                            .ended_at
                            .zip(start)
                            .is_none_or(|(end, start)| end <= start)
                });
                if let Some((status, origin)) = classify(eligible, text(item), start) {
                    handoffs.push(DocumentHandoff {
                        run_id: &run.id,
                        call_id: &call.id,
                        context_id: &item.id,
                        status,
                        origin,
                    });
                }
            }
        }
    }
    handoffs
}

fn classify<'a: 'b, 'b>(
    candidates: impl Iterator<Item = &'b Candidate<'a>>,
    input: Option<&str>,
    start: Option<OffsetDateTime>,
) -> Option<(HandoffStatus, Option<RetrievalOrigin<'a>>)> {
    let mut matched = None;
    let mut unknown_time = false;
    let mut unavailable = false;
    let mut differs = false;
    for candidate in candidates {
        match (input, candidate.text) {
            (Some(input), Some(text)) if input == text => {
                if candidate.ended_at.is_none() || start.is_none() {
                    unknown_time = true;
                } else if matched.replace(candidate.origin).is_some() {
                    return Some((HandoffStatus::Ambiguous, None));
                }
            }
            (Some(_), Some(_)) => differs = true,
            _ => unavailable = true,
        }
    }
    if unavailable {
        Some((HandoffStatus::ContentUnavailable, None))
    } else if unknown_time {
        Some((HandoffStatus::TimingUnavailable, None))
    } else if let Some(origin) = matched {
        Some((HandoffStatus::Matched, Some(origin)))
    } else if differs {
        Some((HandoffStatus::ContentDiffers, None))
    } else {
        None
    }
}

fn text(item: &ContextItem) -> Option<&str> {
    item.text
        .as_deref()
        .filter(|text| item.availability == Availability::Included && !text.trim().is_empty())
}

fn instant(value: Option<&str>) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value?, &Rfc3339).ok()
}
