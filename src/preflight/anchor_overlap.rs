//! Measure shared UTF-8 byte ranges among named sources in the validated request.

use crate::{Availability, Call, TextRange};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const EXAMPLE_LIMIT: usize = 8;

/// Byte counts are unions within decoded string fields. Evidence contains offsets and IDs only.
#[derive(Debug, Default, Serialize)]
pub struct AnchorOverlap {
    pub named_bytes: usize,
    pub shared_bytes: usize,
    pub shared_fields: usize,
    pub examples: Vec<SharedRange>,
}

#[derive(Debug, Serialize)]
pub struct SharedRange {
    pub pointer: String,
    pub text_range: TextRange,
    pub source_count: usize,
    /// Two named fragments that cover the entire reported interval.
    pub sources: [NamedRange; 2],
}

#[derive(Debug, Serialize)]
pub struct NamedRange {
    pub source_id: String,
    pub text_range: TextRange,
}

pub(super) fn measure(call: &Call) -> Option<AnchorOverlap> {
    let mut fields = BTreeMap::<_, Vec<_>>::new();
    for item in &call.context {
        if item.availability != Availability::Included {
            continue;
        }
        if let (Some(id), Some(fragment)) = (item.source_id.as_deref(), &item.request_fragment) {
            if fragment.text_range.start_byte >= fragment.text_range.end_byte {
                return None;
            }
            let events = fields.entry(&fragment.pointer).or_default();
            let range = (fragment.text_range.start_byte, fragment.text_range.end_byte);
            events.push((range.0, true, id, range));
            events.push((range.1, false, id, range));
        }
    }
    if fields.is_empty() {
        return None;
    }
    let mut report = AnchorOverlap::default();
    for (pointer, mut events) in fields {
        events.sort_unstable();
        // Identical declarations from one source describe a single interval.
        events.dedup();
        let mut active = BTreeMap::<&str, BTreeSet<(usize, usize)>>::new();
        let mut previous = 0;
        let mut shared = false;
        for (offset, starts, id, range) in events {
            let bytes = offset - previous;
            if !active.is_empty() {
                report.named_bytes += bytes;
            }
            if bytes > 0 && active.len() > 1 {
                report.shared_bytes += bytes;
                if !shared && report.examples.len() < EXAMPLE_LIMIT {
                    let mut sources = active.iter().map(|(&id, ranges)| {
                        let &(start_byte, end_byte) =
                            ranges.first().expect("active source has a range");
                        NamedRange {
                            source_id: id.to_owned(),
                            text_range: TextRange {
                                start_byte,
                                end_byte,
                            },
                        }
                    });
                    report.examples.push(SharedRange {
                        pointer: pointer.clone(),
                        text_range: TextRange {
                            start_byte: previous,
                            end_byte: offset,
                        },
                        source_count: active.len(),
                        sources: [sources.next()?, sources.next()?],
                    });
                }
                shared = true;
            }
            if starts {
                active.entry(id).or_default().insert(range);
            } else {
                let ranges = active.get_mut(id)?;
                ranges.remove(&range);
                if ranges.is_empty() {
                    active.remove(id);
                }
            }
            previous = offset;
        }
        report.shared_fields += usize::from(shared);
    }
    Some(report)
}
