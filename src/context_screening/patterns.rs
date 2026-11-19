//! Find recurring screening suggestions and open the original source pairs.

use super::{recorded, Assessment, Pair, Report, Scorer, Source, Suggestion, Verification};
use crate::{
    replay::sha256,
    scoped_calls::{self, conversation_key, next_offset, ScopedCall, Selection},
    turn_evidence::{excerpt, inspector_path},
    Call, Run,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

/// Search verified context screens in one or two recorded scopes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    #[serde(default)]
    selection: Selection,
    compare: Option<Selection>,
    source_id: Option<String>,
    relation: Option<Suggestion>,
    #[serde(default)]
    min_conversations: usize,
    group: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    compare_offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct SourceKey<'a> {
    source_id: Option<&'a str>,
    source_version: Option<&'a str>,
    kind: &'a str,
    role: Option<&'a str>,
    text_sha256: &'a str,
}

impl<'a> From<&'a Source> for SourceKey<'a> {
    fn from(source: &'a Source) -> Self {
        Self {
            source_id: source.source_id.as_deref(),
            source_version: source.source_version.as_deref(),
            kind: &source.kind,
            role: source.role.as_deref(),
            text_sha256: &source.text_sha256,
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct PairKey<'a> {
    sources: [SourceKey<'a>; 2],
    scorer: Option<&'a Scorer>,
}

#[derive(Clone, Copy)]
struct Occurrence<'a> {
    run: &'a Run,
    call: &'a Call,
    time: Option<OffsetDateTime>,
    sources: [&'a Source; 2],
    pair: &'a Pair,
}

impl Occurrence<'_> {
    fn evidence(&self) -> Value {
        let [left, right] = self.sources;
        let anchor = json!([
            left.context_id,
            left.text_sha256,
            right.context_id,
            right.text_sha256
        ])
        .to_string();
        let anchor = utf8_percent_encode(&anchor, NON_ALPHANUMERIC);
        let path = inspector_path(&self.run.id, &self.call.id, None);
        json!({
            "run_id": self.run.id, "call_id": self.call.id, "trace_source": self.run.source,
            "organization_id": self.run.organization_id, "user_id": self.run.user_id,
            "conversation_id": self.run.conversation_id, "turn_id": self.call.turn_id,
            "started_at": self.call.started_at, "suggestion": self.pair.suggestion,
            "assessment": self.pair.assessment, "sources": self.sources,
            "inspector_path": format!("{path}&screen_pair={anchor}"),
        })
    }

    fn has_flag(&self, relation: Option<Suggestion>) -> bool {
        self.pair
            .suggestion
            .is_some_and(|suggestion| relation.is_none_or(|filter| filter == suggestion))
    }
}

#[derive(Default)]
struct Group<'a> {
    primary: Vec<Occurrence<'a>>,
    comparison: Vec<Occurrence<'a>>,
}

#[derive(Serialize)]
struct Counts {
    pairs: usize,
    screened_calls: usize,
    assessed_calls: usize,
    unscored_calls: usize,
    flagged_calls: usize,
    flagged_conversations: usize,
    flagged_organizations: usize,
}

fn summarize(rows: &[Occurrence<'_>], relation: Option<Suggestion>) -> Option<Counts> {
    if rows.is_empty() {
        return None;
    }
    let mut screened = BTreeSet::new();
    let mut assessed = BTreeSet::new();
    let mut flagged = BTreeSet::new();
    let mut conversations = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    for row in rows {
        let call = (&row.run.id, &row.call.id);
        screened.insert(call);
        if !matches!(row.pair.assessment, Assessment::Unscored { .. }) {
            assessed.insert(call);
        }
        if row.has_flag(relation) {
            flagged.insert(call);
            conversations.extend(conversation_key(row.run));
            organizations.extend(row.run.organization_id.as_deref());
        }
    }
    Some(Counts {
        pairs: rows.len(),
        screened_calls: screened.len(),
        assessed_calls: assessed.len(),
        unscored_calls: screened.len() - assessed.len(),
        flagged_calls: flagged.len(),
        flagged_conversations: conversations.len(),
        flagged_organizations: organizations.len(),
    })
}

struct GroupRow<'a> {
    id: String,
    key: PairKey<'a>,
    group: Group<'a>,
    primary: Option<Counts>,
    comparison: Option<Counts>,
}

impl GroupRow<'_> {
    fn rank(&self) -> (usize, usize) {
        self.primary.iter().chain(&self.comparison).fold(
            (0, 0),
            |(conversations, calls), counts| {
                (
                    conversations.max(counts.flagged_conversations),
                    calls.max(counts.flagged_calls),
                )
            },
        )
    }

    fn summary(&self, include_excerpts: bool, relation: Option<Suggestion>) -> Value {
        let excerpts = include_excerpts.then(|| {
            let row = self
                .group
                .primary
                .first()
                .or_else(|| self.group.comparison.first())
                .expect("nonempty group");
            let mut sources = row.sources;
            sources.sort_by_key(|&source| SourceKey::from(source));
            sources.map(|source| {
                row.call
                    .context
                    .iter()
                    .find(|item| item.id == source.context_id)
                    .and_then(|item| item.text.as_deref())
                    .map(excerpt)
            })
        });
        let suggestions: BTreeSet<_> = self
            .group
            .primary
            .iter()
            .chain(&self.group.comparison)
            .filter(|row| row.has_flag(relation))
            .filter_map(|row| row.pair.suggestion)
            .collect();
        json!({
            "key": self.id, "sources": self.key.sources, "scorer": self.key.scorer,
            "suggestions": suggestions, "excerpts": excerpts, "primary": self.primary, "comparison": self.comparison,
        })
    }
}

/// Compare flagged calls and screening coverage. Pass already scoped runs.
/// Excerpts are optional and are intended for the local inspector.
pub fn query<'a>(
    request: &Request,
    runs: impl IntoIterator<Item = &'a Run>,
    include_excerpts: bool,
) -> Result<Value, String> {
    if request.version != 1 || !(1..=50).contains(&request.limit) {
        return Err("Use screening pattern version 1 and a page size from 1 to 50.".into());
    }
    let mut groups = BTreeMap::<_, Group<'_>>::new();
    scoped_calls::visit(
        &request.selection,
        request.compare.as_ref(),
        runs,
        |ScopedCall {
             run,
             call,
             time,
             primary,
             comparison,
         }| {
            let Some(screen) = recorded(&run.id, call) else {
                return Ok(());
            };
            let Verification::Matched { report } = screen.verification else {
                return Ok(());
            };
            for pair in &report.pairs {
                let sources = pair.sources.map(|index| &report.sources[index]);
                if request.source_id.as_deref().is_some_and(|source| {
                    !sources
                        .iter()
                        .any(|candidate| candidate.source_id.as_deref() == Some(source))
                }) {
                    continue;
                }
                let Some(scorer) = comparable_scorer(report, pair) else {
                    continue;
                };
                let mut identity = sources.map(SourceKey::from);
                identity.sort_unstable();
                let group = groups
                    .entry(PairKey {
                        sources: identity,
                        scorer,
                    })
                    .or_default();
                let row = Occurrence {
                    run,
                    call,
                    time,
                    sources,
                    pair,
                };
                if primary {
                    group.primary.push(row);
                }
                if comparison {
                    group.comparison.push(row);
                }
            }
            Ok(())
        },
    )?;
    let mut rows = Vec::new();
    for (key, group) in groups {
        let primary = summarize(&group.primary, request.relation);
        let comparison = summarize(&group.comparison, request.relation);
        let mut counts = primary.iter().chain(&comparison);
        if !counts.clone().any(|side| side.flagged_calls > 0)
            || !counts.any(|side| side.flagged_conversations >= request.min_conversations)
        {
            continue;
        }
        let identity = serde_json::to_vec(&("nexusweave.screening_pattern.v1", &key))
            .map_err(|error| error.to_string())?;
        let id = sha256(&identity);
        if request.group.as_ref().is_some_and(|group| group != &id) {
            continue;
        }
        rows.push(GroupRow {
            id,
            key,
            group,
            primary,
            comparison,
        });
    }
    let compared = request.compare.is_some();
    if request.group.is_some() {
        let mut row = rows
            .pop()
            .ok_or("The screened pair group is unavailable in this scope.")?;
        return Ok(json!({
            "kind": "screening_pattern_evidence", "group": row.summary(include_excerpts, request.relation),
            "compared": compared, "relation": request.relation,
            "primary": occurrence_page(&mut row.group.primary, request.offset, request.limit),
            "comparison": compared.then(|| occurrence_page(&mut row.group.comparison, request.compare_offset, request.limit)),
        }));
    }
    rows.sort_by(|left, right| {
        right
            .rank()
            .cmp(&left.rank())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(json!({
        "kind": "screening_pattern_groups", "total_groups": rows.len(),
        "compared": compared, "relation": request.relation,
        "groups": rows.iter().skip(request.offset).take(request.limit).map(|row| row.summary(include_excerpts, request.relation)).collect::<Vec<_>>(),
        "next_offset": next_offset(request.offset, request.limit, rows.len()),
    }))
}

fn comparable_scorer<'a>(report: &'a Report, pair: &Pair) -> Option<Option<&'a Scorer>> {
    match pair.assessment {
        Assessment::Exact => Some(None),
        _ => report.scorer.as_ref().map(Some),
    }
}

fn occurrence_page(items: &mut [Occurrence<'_>], offset: usize, limit: usize) -> Value {
    items.sort_by_key(|item| {
        (
            item.time.is_none(),
            item.time,
            &item.run.id,
            &item.call.id,
            item.pair.sources,
        )
    });
    json!({
        "total": items.len(),
        "rows": items.iter().skip(offset).take(limit).map(Occurrence::evidence).collect::<Vec<_>>(),
        "next_offset": next_offset(offset, limit, items.len()),
    })
}
