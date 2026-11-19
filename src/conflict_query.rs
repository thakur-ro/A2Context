//! Read-only breakdowns of recorded rule conflicts.

use crate::{
    applicable_rules, compare_rules, Call, ContextItem, Decision, DecisionScope, FindingKind, Run,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

pub type GroupKey = Vec<Vec<Option<String>>>;
type Conversation<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSpec {
    pub version: u32,
    #[serde(default)]
    pub scope: Scope,
    pub group_by: Vec<Dimension>,
    #[serde(default)]
    pub filters: Filters,
    pub group: Option<GroupKey>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    #[default]
    Workspace,
    Organization {
        organization_id: String,
    },
    User {
        organization_id: Option<String>,
        user_id: String,
    },
}

impl Scope {
    pub fn includes(&self, run: &Run) -> bool {
        match self {
            Self::Workspace => true,
            Self::Organization { organization_id } => {
                run.organization_id.as_ref() == Some(organization_id)
            }
            Self::User {
                organization_id,
                user_id,
            } => &run.organization_id == organization_id && run.user_id.as_ref() == Some(user_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Organization,
    User,
    Conversation,
    Model,
    SourceVersion,
    Rule,
    Day,
    TraceSource,
}

impl Dimension {
    fn key_width(self) -> usize {
        match self {
            Self::User | Self::SourceVersion | Self::Rule => 2,
            Self::Conversation => 4,
            _ => 1,
        }
    }

    fn key(
        self,
        run: &Run,
        call: &Call,
        rule: &DecisionScope,
        source: Option<&ContextItem>,
        day: Option<&str>,
    ) -> Vec<Option<String>> {
        let org = || run.organization_id.clone();
        let user = || run.user_id.clone();
        match self {
            Self::Organization => vec![org()],
            Self::User => vec![org(), user()],
            Self::Conversation => vec![
                Some(run.source.clone()),
                org(),
                user(),
                run.conversation_id.clone(),
            ],
            Self::Model => vec![call.model.clone()],
            Self::SourceVersion => vec![
                source.and_then(|item| item.source_id.clone()),
                source.and_then(|item| item.source_version.clone()),
            ],
            Self::Rule => vec![Some(rule.key.clone()), Some(rule.scope.clone())],
            Self::Day => vec![day.map(str::to_owned)],
            Self::TraceSource => vec![Some(run.source.clone())],
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    pub rule: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Counts {
    scoped_decisions: usize,
    comparable_decisions: usize,
    conflicting_decisions: usize,
    affected_conversations: usize,
    conflicts_without_conversation: usize,
}

#[derive(Default)]
struct Counter<'a> {
    counts: Counts,
    conversations: BTreeSet<Conversation<'a>>,
}

impl<'a> Counter<'a> {
    fn add(&mut self, run: &'a Run, comparable: bool, conflict: bool) {
        self.counts.scoped_decisions += 1;
        self.counts.comparable_decisions += usize::from(comparable);
        if conflict {
            self.counts.conflicting_decisions += 1;
            if let Some(thread) = &run.conversation_id {
                self.conversations.insert((
                    &run.source,
                    run.organization_id.as_deref(),
                    run.user_id.as_deref(),
                    thread,
                ));
                self.counts.affected_conversations = self.conversations.len();
            } else {
                self.counts.conflicts_without_conversation += 1;
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Group {
    key: GroupKey,
    #[serde(flatten)]
    counts: Counts,
}

#[derive(Debug, Serialize)]
pub struct Evidence<'a> {
    run_id: &'a str,
    call_id: &'a str,
    decision_id: &'a str,
    decision_label: &'a str,
    trace_source: &'a str,
    organization_id: Option<&'a str>,
    user_id: Option<&'a str>,
    conversation_id: Option<&'a str>,
    turn_id: Option<&'a str>,
    started_at: Option<&'a str>,
}

impl<'a> Evidence<'a> {
    fn new(run: &'a Run, call: &'a Call, decision: &'a Decision) -> Self {
        Self {
            run_id: &run.id,
            call_id: &call.id,
            decision_id: &decision.id,
            decision_label: &decision.label,
            trace_source: &run.source,
            organization_id: run.organization_id.as_deref(),
            user_id: run.user_id.as_deref(),
            conversation_id: run.conversation_id.as_deref(),
            turn_id: call.turn_id.as_deref(),
            started_at: call.started_at.as_deref(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Page<'a> {
    Groups {
        totals: Counts,
        rows: Vec<Group>,
        next_offset: Option<usize>,
    },
    Evidence {
        rows: Vec<Evidence<'a>>,
        next_offset: Option<usize>,
    },
}

pub(crate) fn timestamp(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| "Expected an RFC 3339 timestamp.".into())
}

pub(crate) struct TimeWindow {
    after: Option<OffsetDateTime>,
    before: Option<OffsetDateTime>,
}

impl TimeWindow {
    pub(crate) fn parse(after: Option<&str>, before: Option<&str>) -> Result<Self, String> {
        let after = after.map(timestamp).transpose()?;
        let before = before.map(timestamp).transpose()?;
        if after
            .zip(before)
            .is_some_and(|(after, before)| after >= before)
        {
            return Err("The start of the time window must precede its end.".into());
        }
        Ok(Self { after, before })
    }

    pub(crate) fn includes(&self, time: Option<OffsetDateTime>) -> bool {
        self.after
            .is_none_or(|start| time.is_some_and(|time| time >= start))
            && self
                .before
                .is_none_or(|end| time.is_some_and(|time| time < end))
    }
}

/// Query normalized runs with unique run IDs, as required by the inspection format.
pub fn query<'a>(
    spec: &ViewSpec,
    runs: impl IntoIterator<Item = &'a Run>,
) -> Result<Page<'a>, String> {
    if spec.version != 1 {
        return Err("Conflict views require version 1.".into());
    }
    let dimensions = &spec.group_by;
    if dimensions.is_empty()
        || dimensions.len() > 3
        || dimensions.iter().collect::<BTreeSet<_>>().len() != dimensions.len()
    {
        return Err("Choose one to three distinct grouping fields.".into());
    }
    if !(1..=100).contains(&spec.limit) {
        return Err("Choose a page size from 1 to 100.".into());
    }
    if let Some(key) = &spec.group {
        if key.len() != dimensions.len()
            || key
                .iter()
                .zip(dimensions)
                .any(|(part, dimension)| part.len() != dimension.key_width())
        {
            return Err("The selected group key does not match the grouping fields.".into());
        }
    }
    let window = TimeWindow::parse(
        spec.filters.after.as_deref(),
        spec.filters.before.as_deref(),
    )?;

    let mut groups = BTreeMap::<GroupKey, Counter>::new();
    let mut totals = Counter::default();
    let mut evidence = Vec::new();
    let mut matches = 0;
    let by_source = dimensions.contains(&Dimension::SourceVersion);
    for run in runs.into_iter().filter(|run| spec.scope.includes(run)) {
        for call in &run.calls {
            let time = call
                .started_at
                .as_deref()
                .and_then(|value| timestamp(value).ok());
            if !window.includes(time) {
                continue;
            }
            let day = dimensions
                .contains(&Dimension::Day)
                .then(|| time.map(|time| time.to_offset(UtcOffset::UTC).date().to_string()))
                .flatten();
            for decision in &call.decisions {
                let Some(rule) = &decision.applies_to else {
                    continue;
                };
                if spec
                    .filters
                    .rule
                    .as_ref()
                    .is_some_and(|key| key != &rule.key)
                {
                    continue;
                }
                let rules: Vec<_> = applicable_rules(call, rule).collect();
                let mut known = rules
                    .iter()
                    .filter(|(_, rule)| rule.value.is_some())
                    .peekable();
                let first = known.next();
                let comparable = known.peek().is_some();
                let conflict = first.is_some_and(|(_, first)| {
                    known.any(|(_, rule)| compare_rules(first, rule) == FindingKind::Conflict)
                });
                let key_for = |item| {
                    dimensions
                        .iter()
                        .map(|dimension| dimension.key(run, call, rule, item, day.as_deref()))
                        .collect::<GroupKey>()
                };
                let mut memberships = BTreeMap::new();
                if by_source && !rules.is_empty() {
                    for (item, rule) in rules {
                        memberships
                            .entry(key_for(Some(item)))
                            .and_modify(|known| *known |= rule.value.is_some())
                            .or_insert(rule.value.is_some());
                    }
                } else {
                    memberships.insert(key_for(None), true);
                }
                if let Some(group) = &spec.group {
                    if conflict && memberships.get(group) == Some(&true) {
                        if matches >= spec.offset && evidence.len() < spec.limit {
                            evidence.push(Evidence::new(run, call, decision));
                        }
                        matches += 1;
                    }
                } else {
                    totals.add(run, comparable, conflict);
                    for (key, has_value) in memberships {
                        groups.entry(key).or_default().add(
                            run,
                            comparable && has_value,
                            conflict && has_value,
                        );
                    }
                }
            }
        }
    }
    let next = spec.offset.saturating_add(spec.limit);
    if spec.group.is_some() {
        return Ok(Page::Evidence {
            rows: evidence,
            next_offset: (next < matches).then_some(next),
        });
    }
    let size = groups.len();
    let mut rows: Vec<_> = groups
        .into_iter()
        .map(|(key, counter)| Group {
            key,
            counts: counter.counts,
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.counts.conflicting_decisions));
    let rows = rows
        .into_iter()
        .skip(spec.offset)
        .take(spec.limit)
        .collect();
    Ok(Page::Groups {
        totals: totals.counts,
        rows,
        next_offset: (next < size).then_some(next),
    })
}
