//! Read-only breakdowns of recorded decisions and verified preflight rule checks.

use crate::{
    applicable_rules, compare_rules, preflight, Call, ContextItem, Decision, FindingKind, Rule, Run,
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
    pub phase: Phase,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Decision,
    Preflight,
}

impl Phase {
    fn is_decision(&self) -> bool {
        matches!(self, Self::Decision)
    }
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
        rule: (&str, &str),
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
            Self::Rule => vec![Some(rule.0.to_owned()), Some(rule.1.to_owned())],
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

#[derive(Debug, Serialize)]
pub struct Counts {
    #[serde(flatten)]
    units: Units,
    affected_conversations: usize,
    conflicts_without_conversation: usize,
}

impl Default for Counts {
    fn default() -> Self {
        Counter::default().counts(Phase::Decision)
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Units {
    Decisions {
        scoped_decisions: usize,
        comparable_decisions: usize,
        conflicting_decisions: usize,
    },
    Preflight {
        scoped_checks: usize,
        comparable_checks: usize,
        conflicting_checks: usize,
    },
}

#[derive(Default)]
struct Counter<'a> {
    scoped: usize,
    comparable: usize,
    conflicting: usize,
    conflicts_without_conversation: usize,
    conversations: BTreeSet<Conversation<'a>>,
}

impl<'a> Counter<'a> {
    fn add(&mut self, run: &'a Run, comparable: bool, conflict: bool) {
        self.scoped += 1;
        self.comparable += usize::from(comparable);
        if conflict {
            self.conflicting += 1;
            if let Some(thread) = &run.conversation_id {
                self.conversations.insert((
                    &run.source,
                    run.organization_id.as_deref(),
                    run.user_id.as_deref(),
                    thread,
                ));
            } else {
                self.conflicts_without_conversation += 1;
            }
        }
    }

    fn counts(self, phase: Phase) -> Counts {
        let units = match phase {
            Phase::Decision => Units::Decisions {
                scoped_decisions: self.scoped,
                comparable_decisions: self.comparable,
                conflicting_decisions: self.conflicting,
            },
            Phase::Preflight => Units::Preflight {
                scoped_checks: self.scoped,
                comparable_checks: self.comparable,
                conflicting_checks: self.conflicting,
            },
        };
        Counts {
            units,
            affected_conversations: self.conversations.len(),
            conflicts_without_conversation: self.conflicts_without_conversation,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_scope: Option<(&'a str, &'a str)>,
    trace_source: &'a str,
    organization_id: Option<&'a str>,
    user_id: Option<&'a str>,
    conversation_id: Option<&'a str>,
    turn_id: Option<&'a str>,
    started_at: Option<&'a str>,
}

impl<'a> Evidence<'a> {
    fn new(run: &'a Run, call: &'a Call, check: &RuleCheck<'a>) -> Self {
        Self {
            run_id: &run.id,
            call_id: &call.id,
            decision_id: check.decision.map(|decision| decision.id.as_str()),
            decision_label: check.decision.map(|decision| decision.label.as_str()),
            rule_scope: check.decision.is_none().then_some(check.scope),
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
        #[serde(skip_serializing_if = "Phase::is_decision")]
        phase: Phase,
        totals: Counts,
        rows: Vec<Group>,
        next_offset: Option<usize>,
    },
    Evidence {
        #[serde(skip_serializing_if = "Phase::is_decision")]
        phase: Phase,
        rows: Vec<Evidence<'a>>,
        next_offset: Option<usize>,
    },
}

struct RuleCheck<'a> {
    decision: Option<&'a Decision>,
    scope: (&'a str, &'a str),
    rules: Vec<(&'a ContextItem, &'a Rule)>,
}

fn visit_rule_checks<'a>(
    call: &'a Call,
    phase: Phase,
    rule_filter: Option<&str>,
    mut visit: impl FnMut(RuleCheck<'a>),
) {
    let matches = |key: &str| rule_filter.is_none_or(|filter| filter == key);
    match phase {
        Phase::Decision => {
            for decision in &call.decisions {
                let Some(scope) = decision
                    .applies_to
                    .as_ref()
                    .filter(|scope| matches(&scope.key))
                else {
                    continue;
                };
                visit(RuleCheck {
                    decision: Some(decision),
                    scope: (&scope.key, &scope.scope),
                    rules: applicable_rules(call, scope).collect(),
                });
            }
        }
        Phase::Preflight => {
            let Some(receipt) = preflight::verified_receipt(call) else {
                return;
            };
            let mut scopes = BTreeMap::<(&str, &str), Vec<_>>::new();
            if let Some(scope) = receipt.scope.as_ref().filter(|scope| matches(&scope.key)) {
                scopes.insert((&scope.key, &scope.scope), Vec::new());
            }
            for (item, rule) in preflight::rules(call, receipt.scope.as_ref())
                .filter(|(_, rule)| matches(&rule.key))
            {
                scopes
                    .entry((&rule.key, &rule.scope))
                    .or_default()
                    .push((item, rule));
            }
            for (scope, rules) in scopes {
                visit(RuleCheck {
                    decision: None,
                    scope,
                    rules,
                });
            }
        }
    }
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
            visit_rule_checks(call, spec.phase, spec.filters.rule.as_deref(), |check| {
                let mut known = check
                    .rules
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
                        .map(|dimension| {
                            dimension.key(run, call, check.scope, item, day.as_deref())
                        })
                        .collect::<GroupKey>()
                };
                let mut memberships = BTreeMap::new();
                if by_source && !check.rules.is_empty() {
                    for (item, rule) in &check.rules {
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
                            evidence.push(Evidence::new(run, call, &check));
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
            });
        }
    }
    let next = spec.offset.saturating_add(spec.limit);
    if spec.group.is_some() {
        return Ok(Page::Evidence {
            phase: spec.phase,
            rows: evidence,
            next_offset: (next < matches).then_some(next),
        });
    }
    let size = groups.len();
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(_, counter)| std::cmp::Reverse(counter.conflicting));
    let rows = groups
        .into_iter()
        .skip(spec.offset)
        .take(spec.limit)
        .map(|(key, counter)| Group {
            key,
            counts: counter.counts(spec.phase),
        })
        .collect();
    Ok(Page::Groups {
        phase: spec.phase,
        totals: totals.counts(spec.phase),
        rows,
        next_offset: (next < size).then_some(next),
    })
}
