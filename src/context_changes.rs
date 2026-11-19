//! Align recorded context and index transitions between consecutive input calls.

use crate::{
    context_links::{result_invocation, tool_definition_name},
    decision_changes::{compare_decisions, DecisionChange},
    replay::{capture_availability, has_input_profile, CaptureAvailability},
    Call, ContextItem, Run,
};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub use crate::decision_changes::{compare_outcomes, OutcomeStatus};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct CallLocation<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Scope<'a> {
    pub trace_source: &'a str,
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
}

impl<'a> From<&'a Run> for Scope<'a> {
    fn from(run: &'a Run) -> Self {
        Self {
            trace_source: &run.source,
            organization_id: run.organization_id.as_deref(),
            user_id: run.user_id.as_deref(),
            conversation_id: run.conversation_id.as_deref().filter(|id| !id.is_empty()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Identity<'a> {
    Source { id: &'a str },
    Document { id: &'a str },
    Tool { name: Cow<'a, str> },
    ToolResult { invocation_id: &'a str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Matching,
    Changed,
    CaptureChanged,
    Incomplete,
    First,
    Second,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Difference {
    Version,
    Authority,
    Kind,
    Role,
    ContentReference,
    DocumentId,
    RequestLocation,
    CaptureStatus,
    Rule,
    Text,
}

#[derive(Debug, Serialize)]
pub struct Alignment<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity<'a>>,
    pub context_ids: [Vec<&'a str>; 2],
    pub status: Status,
    pub differences: Vec<Difference>,
}

#[derive(Debug, Serialize)]
pub struct Timeline<'a> {
    #[serde(flatten)]
    pub scope: Scope<'a>,
    pub runs: Vec<&'a str>,
    pub has_timestamps: bool,
    pub calls: Vec<CallLocation<'a>>,
}

#[derive(Debug, Serialize)]
pub struct Transition<'a> {
    pub before: CallLocation<'a>,
    pub after: CallLocation<'a>,
    #[serde(flatten)]
    pub scope: Scope<'a>,
    pub before_started_at: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub before_turn_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_change: Option<DecisionChange<'a>>,
    #[serde(flatten)]
    pub result: TransitionResult<'a>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TransitionResult<'a> {
    Compared { changes: Vec<Alignment<'a>> },
    AmbiguousOrder,
    MissingContext,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PatternKey<'a> {
    pub identity: Identity<'a>,
    pub adapter: Option<&'a str>,
    pub status: Status,
    pub versions: [Vec<&'a str>; 2],
    pub captures: [Vec<CaptureAvailability>; 2],
    pub differences: Vec<Difference>,
}

#[derive(Debug, Serialize)]
pub struct Pattern<'a> {
    #[serde(flatten)]
    pub key: PatternKey<'a>,
    pub source_observation_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earlier_state_index: Option<usize>,
    pub occurrences: Vec<Occurrence<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SourceKey<'a> {
    pub identity: Identity<'a>,
    pub adapter: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct EarlierEntry<'a> {
    pub version: Option<&'a str>,
    pub capture: CaptureAvailability,
}

#[derive(Debug, Serialize)]
pub struct EarlierState<'a> {
    pub entries: Vec<EarlierEntry<'a>>,
    pub pairs: Vec<SourcePair<'a>>,
}

#[derive(Debug, Serialize)]
pub struct SourcePair<'a> {
    pub transition_index: usize,
    pub context_ids: [Vec<&'a str>; 2],
    pub status: Status,
}

#[derive(Debug, Serialize)]
pub struct SourceObservation<'a> {
    #[serde(flatten)]
    pub source: SourceKey<'a>,
    pub earlier_states: Vec<EarlierState<'a>>,
}

#[derive(Debug, Serialize)]
pub struct Occurrence<'a> {
    pub transition_index: usize,
    pub change_index: usize,
    #[serde(flatten)]
    pub scope: Scope<'a>,
    pub started_at: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct ContextHistory<'a> {
    pub context_timelines: Vec<Timeline<'a>>,
    pub context_transitions: Vec<Transition<'a>>,
    pub context_change_patterns: Vec<Pattern<'a>>,
    pub context_source_observations: Vec<SourceObservation<'a>>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Key<'a> {
    Named(Identity<'a>),
    Exact(&'a str, Option<&'a str>, &'a str),
    Unpaired(usize, &'a str),
}

struct Aligned<'a> {
    identity: Option<Identity<'a>>,
    items: [Vec<&'a ContextItem>; 2],
    status: Status,
    differences: Vec<Difference>,
}

impl<'a> From<Aligned<'a>> for Alignment<'a> {
    fn from(group: Aligned<'a>) -> Self {
        let context_ids = group.context_ids();
        Self {
            identity: group.identity,
            context_ids,
            status: group.status,
            differences: group.differences,
        }
    }
}

impl<'a> Aligned<'a> {
    fn context_ids(&self) -> [Vec<&'a str>; 2] {
        std::array::from_fn(|side| {
            self.items[side]
                .iter()
                .map(|item| item.id.as_str())
                .collect()
        })
    }
}

pub fn align<'a>(before: (&'a Run, &'a Call), after: (&'a Run, &'a Call)) -> Vec<Alignment<'a>> {
    align_items(before, after)
        .into_iter()
        .map(Alignment::from)
        .collect()
}

fn align_items<'a>(before: (&'a Run, &'a Call), after: (&'a Run, &'a Call)) -> Vec<Aligned<'a>> {
    let same_trace = before.0.id == after.0.id
        && before.1.trace_id.is_some()
        && before.1.trace_id == after.1.trace_id;
    let same_source = before.0.source == after.0.source;
    let mut groups: Vec<(Option<Identity<'a>>, [Vec<&'a ContextItem>; 2])> = Vec::new();
    let mut lookup = HashMap::new();
    for (side, (_, call)) in [before, after].into_iter().enumerate() {
        for item in &call.context {
            let identity = identified(item, call, same_trace, same_source);
            let key = if let Some(identity) = &identity {
                Key::Named(identity.clone())
            } else if let Some(text) = available_text(item) {
                Key::Exact(&item.kind, item.role.as_deref(), text)
            } else {
                Key::Unpaired(side, &item.id)
            };
            let index = *lookup.entry(key).or_insert_with(|| {
                groups.push((identity, [Vec::new(), Vec::new()]));
                groups.len() - 1
            });
            groups[index].1[side].push(item);
        }
    }
    groups
        .into_iter()
        .map(|(identity, items)| {
            let (status, differences) = alignment_status(&items);
            Aligned {
                identity,
                items,
                status,
                differences,
            }
        })
        .collect()
}

fn available_text(item: &ContextItem) -> Option<&str> {
    item.text
        .as_deref()
        .filter(|_| capture_availability(item) == CaptureAvailability::Available)
}

fn identified<'a>(
    item: &'a ContextItem,
    call: &'a Call,
    same_trace: bool,
    same_source: bool,
) -> Option<Identity<'a>> {
    if let Some(id) = item.source_id.as_deref().filter(|id| !id.trim().is_empty()) {
        return Some(Identity::Source { id });
    }
    if same_trace {
        if let Some(document) = &item.document_identity {
            return Some(Identity::Document { id: &document.id });
        }
        if matches!(item.kind.as_str(), "tool_result" | "tool_call_response") {
            if let Some(invocation_id) = result_invocation(call, item).filter(|id| !id.is_empty()) {
                return Some(Identity::ToolResult { invocation_id });
            }
        }
    }
    if same_source && item.kind == "tool_definition" {
        return tool_definition_name(item, available_text(item)?)
            .filter(|(name, _)| !name.trim().is_empty())
            .map(|(name, _)| Identity::Tool { name });
    }
    None
}

fn alignment_status(items: &[Vec<&ContextItem>; 2]) -> (Status, Vec<Difference>) {
    let (left, right) = match (items[0].as_slice(), items[1].as_slice()) {
        (_, []) => return (Status::First, Vec::new()),
        ([], _) => return (Status::Second, Vec::new()),
        ([left], [right]) => (left, right),
        _ => return (Status::Ambiguous, Vec::new()),
    };
    let before = capture_availability(left);
    let after = capture_availability(right);
    let available =
        before == CaptureAvailability::Available && after == CaptureAvailability::Available;
    let compared = [
        (
            Difference::Version,
            left.source_version != right.source_version,
        ),
        (Difference::Authority, left.authority != right.authority),
        (Difference::Kind, left.kind != right.kind),
        (Difference::Role, left.role != right.role),
        (
            Difference::ContentReference,
            left.content_reference != right.content_reference,
        ),
        (
            Difference::DocumentId,
            left.document_identity.as_ref().map(|item| &item.id)
                != right.document_identity.as_ref().map(|item| &item.id),
        ),
        (
            Difference::RequestLocation,
            left.request_fragment
                .as_ref()
                .zip(right.request_fragment.as_ref())
                .is_some_and(|(before, after)| before != after),
        ),
        (Difference::CaptureStatus, before != after),
        (Difference::Rule, left.rule != right.rule),
        (Difference::Text, available && left.text != right.text),
    ];
    let differences: Vec<_> = compared
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect();
    let status = if before != after {
        Status::CaptureChanged
    } else if !available {
        Status::Incomplete
    } else if differences.is_empty() {
        Status::Matching
    } else {
        Status::Changed
    };
    (status, differences)
}

#[derive(Clone, Copy)]
struct Step<'a> {
    run: &'a Run,
    call: &'a Call,
    timestamp: Option<i128>,
}

impl<'a> Step<'a> {
    fn location(self) -> CallLocation<'a> {
        CallLocation {
            run_id: &self.run.id,
            call_id: &self.call.id,
        }
    }
}

pub(crate) fn history<'a>(runs: impl Iterator<Item = &'a Run>) -> ContextHistory<'a> {
    let mut threads = BTreeMap::<_, Vec<_>>::new();
    for run in runs {
        let scope = Scope::from(run);
        let thread = scope.conversation_id;
        let key = (
            scope.trace_source,
            scope.organization_id,
            scope.user_id,
            thread,
            thread.is_none().then_some(&run.id),
        );
        threads.entry(key).or_default().push(run);
    }
    let mut context_timelines = Vec::new();
    let mut context_transitions = Vec::new();
    let mut observations = Observations::default();
    for runs in threads.values() {
        let mut steps: Vec<_> = runs
            .iter()
            .flat_map(|&run| {
                run.calls.iter().map(move |call| Step {
                    run,
                    call,
                    timestamp: call
                        .started_at
                        .as_deref()
                        .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
                        .map(OffsetDateTime::unix_timestamp_nanos),
                })
            })
            .collect();
        let has_timestamps = steps.iter().all(|step| step.timestamp.is_some());
        if has_timestamps {
            steps.sort_by_key(|step| step.timestamp);
        }
        context_timelines.push(Timeline {
            scope: Scope::from(runs[0]),
            runs: runs.iter().map(|run| run.id.as_str()).collect(),
            has_timestamps,
            calls: steps.iter().map(|step| step.location()).collect(),
        });
        let inputs: Vec<_> = steps
            .into_iter()
            .filter(|step| {
                let call = step.call;
                call.retrieval.is_none()
                    && has_input_profile(call)
                    && (call.decisions.is_empty()
                        || call
                            .decisions
                            .iter()
                            .any(|decision| decision.kind != "tool_execution"))
            })
            .collect();
        let tied = if has_timestamps && runs.len() > 1 {
            let mut seen = HashMap::new();
            inputs
                .iter()
                .filter_map(|step| {
                    let time = step.timestamp;
                    let previous = seen.insert(time, &step.run.id)?;
                    (previous != &step.run.id).then_some(time)
                })
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        for pair in inputs.windows(2) {
            let [before, after] = [pair[0], pair[1]];
            let scope = Scope::from(after.run);
            let result = if before.run.id != after.run.id
                && (!has_timestamps
                    || tied.contains(&before.timestamp)
                    || tied.contains(&after.timestamp))
            {
                TransitionResult::AmbiguousOrder
            } else if before.call.context.is_empty() || after.call.context.is_empty() {
                TransitionResult::MissingContext
            } else {
                let changes = observations.compare(before, after, context_transitions.len());
                TransitionResult::Compared { changes }
            };
            let decision_change = (!matches!(result, TransitionResult::AmbiguousOrder))
                .then(|| compare_decisions(&before.call.decisions, &after.call.decisions));
            context_transitions.push(Transition {
                before: before.location(),
                after: after.location(),
                scope,
                before_started_at: before.call.started_at.as_deref(),
                started_at: after.call.started_at.as_deref(),
                before_turn_id: before.call.turn_id.as_deref(),
                turn_id: after.call.turn_id.as_deref(),
                decision_change,
                result,
            });
        }
    }
    let (context_source_observations, context_change_patterns) = observations.finish();
    ContextHistory {
        context_timelines,
        context_transitions,
        context_change_patterns,
        context_source_observations,
    }
}

type EarlierEntries<'a> = Vec<EarlierEntry<'a>>;

#[derive(Default)]
struct Observations<'a> {
    sources: BTreeMap<SourceKey<'a>, BTreeMap<EarlierEntries<'a>, Vec<SourcePair<'a>>>>,
    patterns: BTreeMap<(SourceKey<'a>, PatternKey<'a>, EarlierEntries<'a>), Vec<Occurrence<'a>>>,
}

impl<'a> Observations<'a> {
    fn compare(&mut self, before: Step<'a>, after: Step<'a>, index: usize) -> Vec<Alignment<'a>> {
        let scope = Scope::from(after.run);
        let mut changes = Vec::new();
        for group in align_items((before.run, before.call), (after.run, after.call)) {
            let changed = group.status != Status::Matching
                && (group.status != Status::Incomplete || !group.differences.is_empty());
            if let Some(source) = source_key(&group, scope.trace_source) {
                let mut earlier: EarlierEntries<'_> = group.items[0]
                    .iter()
                    .map(|item| EarlierEntry {
                        version: source_version(item),
                        capture: capture_availability(item),
                    })
                    .collect();
                earlier.sort_unstable();
                self.sources
                    .entry(source.clone())
                    .or_default()
                    .entry(earlier.clone())
                    .or_default()
                    .push(SourcePair {
                        transition_index: index,
                        context_ids: group.context_ids(),
                        status: group.status,
                    });
                if changed {
                    self.patterns
                        .entry((source.clone(), pattern_key(&group, &source), earlier))
                        .or_default()
                        .push(Occurrence {
                            transition_index: index,
                            change_index: changes.len(),
                            scope,
                            started_at: after.call.started_at.as_deref(),
                        });
                }
            }
            if changed {
                changes.push(Alignment::from(group));
            }
        }
        changes
    }

    fn finish(self) -> (Vec<SourceObservation<'a>>, Vec<Pattern<'a>>) {
        let used: BTreeSet<_> = self.patterns.keys().map(|(source, _, _)| source).collect();
        let sources: Vec<_> = self
            .sources
            .into_iter()
            .filter(|(source, _)| used.contains(source))
            .map(|(source, earlier_states)| SourceObservation {
                source,
                earlier_states: earlier_states
                    .into_iter()
                    .map(|(entries, pairs)| EarlierState { entries, pairs })
                    .collect(),
            })
            .collect();
        let indices: BTreeMap<_, _> = sources
            .iter()
            .enumerate()
            .map(|(index, item)| (&item.source, index))
            .collect();
        let patterns = self
            .patterns
            .into_iter()
            .map(|((source, key, earlier), occurrences)| {
                let source_observation_index = indices[&source];
                let earlier_state_index = (!earlier.is_empty()).then(|| {
                    sources[source_observation_index]
                        .earlier_states
                        .binary_search_by(|state| state.entries.cmp(&earlier))
                        .expect("earlier state indexed when recording the pattern")
                });
                Pattern {
                    key,
                    source_observation_index,
                    earlier_state_index,
                    occurrences,
                }
            })
            .collect();
        (sources, patterns)
    }
}

fn source_key<'a>(group: &Aligned<'a>, trace_source: &'a str) -> Option<SourceKey<'a>> {
    let identity = group.identity.as_ref()?;
    let adapter = match identity {
        Identity::Source { .. } => None,
        Identity::Tool { .. } => Some(trace_source),
        _ => return None,
    };
    Some(SourceKey {
        identity: identity.clone(),
        adapter,
    })
}

fn source_version(item: &ContextItem) -> Option<&str> {
    item.source_version
        .as_deref()
        .filter(|value| !value.is_empty())
}

fn pattern_key<'a>(group: &Aligned<'a>, source: &SourceKey<'a>) -> PatternKey<'a> {
    let versions = std::array::from_fn(|side| {
        group.items[side]
            .iter()
            .filter_map(|item| source_version(item))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    });
    let captures = std::array::from_fn(|side| {
        group.items[side]
            .iter()
            .map(|item| capture_availability(item))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    });
    PatternKey {
        identity: source.identity.clone(),
        adapter: source.adapter,
        status: group.status,
        versions,
        captures,
        differences: group.differences.clone(),
    }
}
