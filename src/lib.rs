//! Diagnose context in LLM workflows before and after inference.
//!
//! Call [`preflight::check`] on an assembled request to find conflicts in
//! declared rules or [`context_screening::screen_pending`] to compare captured
//! text. For recorded traces, use [`otlp::import`] or [`langsmith::import`]
//! and pass the resulting [`Run`] values to [`inspect`].
//!
//! ## Import a recorded turn
//!
//! This synthetic trace records a 60-day answer and a preflight receipt for
//! the same request. Both sources declare the standard return window. Their
//! different values produce a conflict linked to the recorded call.
//!
//! ```
//! use nexusweave::{inspect, langsmith, preflight::Verification, FindingKind};
//!
//! let trace = include_str!(concat!(
//!     env!("CARGO_MANIFEST_DIR"),
//!     "/examples/preflight-returns-langsmith.json"
//! ));
//! let runs = langsmith::import(&serde_json::from_str(trace)?)?;
//! let inspection = inspect(&runs)?;
//! let check = &inspection.preflight_checks[0];
//!
//! assert_eq!(check.call_id, "returns-run");
//! assert!(matches!(check.verification, Verification::Matched { .. }));
//! assert_eq!(inspection.findings[0].kind, FindingKind::Conflict);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Test how a source affects an outcome
//!
//! - [`replay::candidates::execute`] compares fixed continuations before a decision.
//! - [`replay::shadow::execute`] compares generated proposals before dispatch.
//! - [`replay::execute`] measures how an edit changes a recorded decision in replay.
//! - [`activation`] validates measurements from an instrumented runtime.
//!
//! See the [package guide](https://github.com/thakur-ro/NexusWeave/blob/main/docs/rust-api.md)
//! for input formats and the [architecture guide](https://github.com/thakur-ro/NexusWeave/blob/main/docs/architecture.md)
//! for what each result measures.

pub mod activation;
pub mod conflict_query;
pub mod context_changes;
pub mod context_links;
pub mod context_patterns;
pub mod context_screening;
pub mod decision_changes;
pub mod document_handoffs;
pub mod exgentic;
mod genai;
pub mod langsmith;
mod openinference;
pub mod otlp;
pub mod preflight;
pub mod replay;
pub mod replay_effects;
mod scoped_calls;
pub mod source_profiles;
mod span_metadata;
mod tool_patterns;
pub mod turn_evidence;

pub use tool_patterns::{ToolOccurrence, ToolPattern};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: u32 = 1;

fn canonical_json(value: &Value) -> String {
    let mut ordered = value.clone();
    ordered.sort_all_objects();
    ordered.to_string()
}

/// A normalized trace and the scope identifiers it carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub schema_version: u32,
    pub id: String,
    pub source: String,
    pub organization_id: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub calls: Vec<Call>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TraceSpan>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceSpan {
    pub id: String,
    pub parent_span_id: Option<String>,
    pub name: Option<String>,
    pub started_at: Option<String>,
}

/// One captured inference step, including its context and recorded decisions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub turn_id: Option<String>,
    pub started_at: Option<String>,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval: Option<Retrieval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_request: Option<RecordedRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_metadata: Option<ContextMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_receipt: Option<preflight::CapturedReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_probes: Option<replay::candidates::CapturedReports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_probes: Option<replay::shadow::CapturedReports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_screening: Option<context_screening::CapturedReport>,
    pub context: Vec<ContextItem>,
    pub decisions: Vec<Decision>,
    pub tool_results: Vec<ToolResult>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Retrieval {
    pub label: String,
    pub documents: Vec<ContextItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextMetadata {
    pub evidence: Evidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub body: String,
    pub evidence: Evidence,
}

impl RecordedRequest {
    pub(crate) fn parse(&self) -> Result<Value, String> {
        serde_json::from_str(&self.body)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| "recorded request body must be a JSON object".into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub span_id: Option<String>,
    pub field: String,
    pub message_index: Option<usize>,
    pub part_index: Option<usize>,
}

/// A captured input with its source, availability, and evidence location.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: String,
    pub kind: String,
    pub role: Option<String>,
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_identity: Option<DocumentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fragment: Option<RequestFragment>,
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub authority: Option<String>,
    pub availability: Availability,
    pub evidence: Evidence,
    pub rule: Option<Rule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentIdentity {
    pub id: String,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestFragment {
    pub pointer: String,
    pub text_range: TextRange,
}

impl RequestFragment {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !replay::valid_pointer(&self.pointer)
            || self.text_range.start_byte >= self.text_range.end_byte
        {
            return Err(
                "a request fragment requires a JSON Pointer and a nonempty UTF-8 byte range".into(),
            );
        }
        Ok(())
    }

    pub(crate) fn text<'a>(&self, request: &'a Value) -> Option<&'a str> {
        request
            .pointer(&self.pointer)?
            .as_str()?
            .get(self.text_range.bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRange {
    pub start_byte: usize,
    pub end_byte: usize,
}

impl TextRange {
    pub(crate) fn bytes(self) -> std::ops::Range<usize> {
        self.start_byte..self.end_byte
    }

    pub(crate) fn disjoint(self, other: Self) -> bool {
        self.end_byte <= other.start_byte || other.end_byte <= self.start_byte
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Included,
    Redacted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub key: String,
    pub scope: String,
    pub value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub outcome: Value,
    pub evidence: Evidence,
    pub tool_call_id: Option<String>,
    pub applies_to: Option<DecisionScope>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionScope {
    pub key: String,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub outcome: Option<Value>,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub decision_id: String,
    pub context_id: String,
    pub relation: String,
    pub basis: Basis,
    pub evidence: Option<Evidence>,
    pub excerpt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub run_id: String,
    pub call_id: String,
    pub relationship: Relationship,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Observed,
    Declared,
    Derived,
    Hypothesis,
}

/// Findings and cross-run evidence derived by [`inspect`].
#[derive(Debug, Serialize)]
pub struct Inspection<'a> {
    pub runs: Vec<&'a Run>,
    pub findings: Vec<Finding>,
    pub patterns: Vec<Pattern>,
    pub tool_links: Vec<ToolLink<'a>>,
    pub context_links: Vec<context_links::ContextLink<'a>>,
    pub context_patterns: Vec<context_patterns::ContextPattern<'a>>,
    pub source_profiles: Vec<source_profiles::SourceProfile<'a>>,
    pub capture_profiles: Vec<replay::CallCapture<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preflight_checks: Vec<preflight::RecordedCheck<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidate_probes: Vec<replay::candidates::RecordedProbe<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shadow_probes: Vec<replay::shadow::RecordedProbe<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context_screens: Vec<context_screening::Recorded<'a>>,
    #[serde(flatten)]
    pub context_history: context_changes::ContextHistory<'a>,
    pub document_handoffs: Vec<document_handoffs::DocumentHandoff<'a>>,
    pub tool_patterns: Vec<ToolPattern<'a>>,
}

#[derive(Debug, Serialize)]
pub struct ToolLink<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub decision_id: &'a str,
    pub result_call_id: &'a str,
    pub result: &'a ToolResult,
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub run_id: String,
    pub call_id: String,
    pub decision_id: String,
    pub kind: FindingKind,
    pub key: String,
    pub scope: String,
    pub context_ids: [String; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Conflict,
    Duplicate,
    InsufficientEvidence,
}

fn applicable_rules<'a>(
    call: &'a Call,
    scope: &'a DecisionScope,
) -> impl Iterator<Item = (&'a ContextItem, &'a Rule)> {
    call.context.iter().filter_map(move |item| {
        let rule = item.rule.as_ref()?;
        (rule.key == scope.key && rule.scope == scope.scope).then_some((item, rule))
    })
}

fn compare_rules(left: &Rule, right: &Rule) -> FindingKind {
    match (&left.value, &right.value) {
        (Some(left), Some(right)) if left == right => FindingKind::Duplicate,
        (Some(_), Some(_)) => FindingKind::Conflict,
        _ => FindingKind::InsufficientEvidence,
    }
}

#[derive(Debug, Serialize)]
pub struct Pattern {
    pub organization_id: Option<String>,
    pub user_id: Option<String>,
    pub kind: FindingKind,
    pub key: String,
    pub scope: String,
    pub sources: [(String, String); 2],
    pub affected_runs: usize,
    pub affected_turns: Option<usize>,
    pub affected_conversations: Option<usize>,
    pub occurrences: Vec<usize>,
}

pub fn validate(run: &Run) -> Result<(), String> {
    if run.schema_version != SCHEMA_VERSION {
        return Err(format!("unsupported schema version {}", run.schema_version));
    }
    if run.id.trim().is_empty() || run.source.trim().is_empty() || run.calls.is_empty() {
        return Err("run requires an id, a source, and at least one call".into());
    }
    unique_ids(run.calls.iter().map(|call| call.id.as_str()), "call")?;
    unique_ids(run.spans.iter().map(|span| span.id.as_str()), "trace span")?;
    for call in &run.calls {
        if let Some(request) = &call.recorded_request {
            request
                .parse()
                .map_err(|error| format!("call {}: {error}", call.id))?;
            if request.evidence.field.trim().is_empty() {
                return Err(format!(
                    "call {}: recorded request requires an evidence field",
                    call.id
                ));
            }
        }
        unique_ids(call.context.iter().map(|item| item.id.as_str()), "context")?;
        if let Some(retrieval) = &call.retrieval {
            unique_ids(
                retrieval.documents.iter().map(|item| item.id.as_str()),
                "retrieved document",
            )?;
        }
        unique_ids(
            call.decisions.iter().map(|decision| decision.id.as_str()),
            "decision",
        )?;
        let retrieved = call
            .retrieval
            .iter()
            .flat_map(|retrieval| &retrieval.documents);
        for item in call.context.iter().chain(retrieved) {
            if let Some(fragment) = &item.request_fragment {
                fragment
                    .validate()
                    .map_err(|error| format!("context {}: {error}", item.id))?;
            }
            if item.document_identity.as_ref().is_some_and(|document| {
                document.id.trim().is_empty() || document.evidence.field.trim().is_empty()
            }) {
                return Err(format!(
                    "document identity on {} requires an ID and evidence field",
                    item.id
                ));
            }
            if item.availability != Availability::Included
                && (item.text.is_some()
                    || item.rule.as_ref().is_some_and(|rule| rule.value.is_some()))
            {
                return Err(format!(
                    "unavailable context {} contains evidence text or value",
                    item.id
                ));
            }
        }
        for relationship in &call.relationships {
            validate_relationship(call, relationship)?;
        }
    }
    Ok(())
}

pub fn add_annotations(runs: &mut [Run], annotations: Vec<Annotation>) -> Result<(), String> {
    for run in runs.iter() {
        validate(run)?;
    }
    unique_ids(runs.iter().map(|run| run.id.as_str()), "run")?;
    let mut pending = Vec::with_capacity(annotations.len());
    for annotation in annotations {
        let (run_index, call_index) = runs
            .iter()
            .position(|run| run.id == annotation.run_id)
            .and_then(|index| {
                runs[index]
                    .calls
                    .iter()
                    .position(|call| call.id == annotation.call_id)
                    .map(|call_index| (index, call_index))
            })
            .ok_or_else(|| {
                format!(
                    "unknown annotation target {}/{}",
                    annotation.run_id, annotation.call_id
                )
            })?;
        validate_relationship(&runs[run_index].calls[call_index], &annotation.relationship)?;
        pending.push((run_index, call_index, annotation.relationship));
    }
    for (run_index, call_index, relationship) in pending {
        let call = &mut runs[run_index].calls[call_index];
        if !call.relationships.contains(&relationship) {
            call.relationships.push(relationship);
        }
    }
    Ok(())
}

fn validate_relationship(call: &Call, relationship: &Relationship) -> Result<(), String> {
    let context = call
        .context
        .iter()
        .find(|item| item.id == relationship.context_id);
    if !call
        .decisions
        .iter()
        .any(|item| item.id == relationship.decision_id)
        || context.is_none()
        || relationship.relation.trim().is_empty()
        || (relationship.basis == Basis::Observed && relationship.evidence.is_none())
        || relationship.excerpt.as_ref().is_some_and(|excerpt| {
            excerpt.is_empty()
                || !context
                    .and_then(|item| item.text.as_deref())
                    .is_some_and(|text| text.contains(excerpt))
        })
    {
        return Err(format!("invalid evidence relationship on call {}", call.id));
    }
    Ok(())
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, kind: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() || !seen.insert(id) {
            return Err(format!("invalid or duplicate {kind} id {id:?}"));
        }
    }
    Ok(())
}

/// Derive findings, context links, and cross-run patterns from recorded evidence.
///
/// Identical copies of a run are counted once. The result borrows the input runs.
///
/// # Errors
///
/// Returns an error for invalid evidence or conflicting run IDs.
pub fn inspect(runs: &[Run]) -> Result<Inspection<'_>, String> {
    let mut unique = BTreeMap::new();
    for run in runs {
        validate(run)?;
        if let Some(previous) = unique.insert((&run.source, &run.id), run) {
            if previous != run {
                return Err(format!("different records share run id {}", run.id));
            }
        }
    }
    unique_ids(
        unique.values().map(|run| run.id.as_str()),
        "run across sources",
    )?;
    let mut findings = Vec::new();
    let mut tool_links = Vec::new();
    let mut context_links = Vec::new();
    let mut preflight_checks = Vec::new();
    let mut candidate_probes = Vec::new();
    let mut shadow_probes = Vec::new();
    let mut context_screens = Vec::new();
    let mut groups = BTreeMap::new();

    for run in unique.values() {
        context_links.extend(context_links::link_run(run));
        for (index, call) in run.calls.iter().enumerate() {
            preflight_checks.extend(preflight::recorded(&run.id, call));
            candidate_probes.extend(replay::candidates::recorded(&run.id, call));
            shadow_probes.extend(replay::shadow::recorded(&run.id, call));
            context_screens.extend(context_screening::recorded(&run.id, call));
            for decision in &call.decisions {
                if let Some(id) = &decision.tool_call_id {
                    if let Some((result_call, result)) = nearest_result(run, index, id) {
                        tool_links.push(ToolLink {
                            run_id: &run.id,
                            call_id: &call.id,
                            decision_id: &decision.id,
                            result_call_id: &result_call.id,
                            result,
                        });
                    }
                }
                let Some(scope) = &decision.applies_to else {
                    continue;
                };
                let applicable: Vec<_> = applicable_rules(call, scope).collect();
                for (left_index, (left, left_rule)) in applicable.iter().enumerate() {
                    for (right, right_rule) in &applicable[left_index + 1..] {
                        let kind = compare_rules(left_rule, right_rule);
                        if let Some(sources) = source_pair(left, right) {
                            if kind != FindingKind::InsufficientEvidence {
                                // Across-run and per-user views share the same occurrence.
                                let organization = run.organization_id.as_deref();
                                let user = run.user_id.as_deref();
                                for (organization, user) in
                                    [(None, None), (organization, None), (organization, user)]
                                        .into_iter()
                                        .collect::<BTreeSet<_>>()
                                {
                                    let group = groups
                                        .entry((
                                            organization,
                                            user,
                                            kind,
                                            &scope.key,
                                            &scope.scope,
                                            sources,
                                        ))
                                        .or_insert_with(Vec::new);
                                    group.push((findings.len(), *run, call));
                                }
                            }
                        }
                        findings.push(Finding {
                            run_id: run.id.clone(),
                            call_id: call.id.clone(),
                            decision_id: decision.id.clone(),
                            kind,
                            key: scope.key.clone(),
                            scope: scope.scope.clone(),
                            context_ids: [left.id.clone(), right.id.clone()],
                        });
                    }
                }
            }
        }
    }
    let patterns = groups
        .into_iter()
        .map(
            |((organization, user, kind, key, scope, sources), entries)| {
                let affected_runs = entries
                    .iter()
                    .map(|(_, run, _)| &run.id)
                    .collect::<BTreeSet<_>>()
                    .len();
                let turns: Option<BTreeSet<_>> = entries
                    .iter()
                    .map(|(_, run, call)| {
                        Some((
                            &run.organization_id,
                            &run.user_id,
                            run.conversation_id.as_ref()?,
                            call.turn_id.as_ref()?,
                        ))
                    })
                    .collect();
                let conversations: Option<BTreeSet<_>> = entries
                    .iter()
                    .map(|(_, run, _)| {
                        Some((
                            &run.organization_id,
                            &run.user_id,
                            run.conversation_id.as_ref()?,
                        ))
                    })
                    .collect();
                Pattern {
                    organization_id: organization.map(str::to_owned),
                    user_id: user.map(str::to_owned),
                    kind,
                    key: key.clone(),
                    scope: scope.clone(),
                    sources: sources.map(|(id, version)| (id.to_owned(), version.to_owned())),
                    affected_runs,
                    affected_turns: turns.map(|items| items.len()),
                    affected_conversations: conversations.map(|items| items.len()),
                    occurrences: entries.into_iter().map(|(index, _, _)| index).collect(),
                }
            },
        )
        .collect();
    let context_patterns = context_patterns::group(unique.values().copied(), &context_links);
    Ok(Inspection {
        document_handoffs: document_handoffs::link(unique.values().copied()),
        source_profiles: source_profiles::group(unique.values().copied(), &context_links),
        capture_profiles: replay::capture_profiles(unique.values().copied())?,
        preflight_checks,
        candidate_probes,
        shadow_probes,
        context_screens,
        context_history: context_changes::history(unique.values().copied()),
        context_patterns,
        tool_patterns: tool_patterns::group(unique.values().copied(), &context_links),
        runs: unique.into_values().collect(),
        findings,
        patterns,
        tool_links,
        context_links,
    })
}

fn source_pair<'a>(
    left: &'a ContextItem,
    right: &'a ContextItem,
) -> Option<[(&'a str, &'a str); 2]> {
    let mut sources = [
        (left.source_id.as_deref()?, left.source_version.as_deref()?),
        (
            right.source_id.as_deref()?,
            right.source_version.as_deref()?,
        ),
    ];
    sources.sort_unstable();
    Some(sources)
}

fn nearest_result<'a>(run: &'a Run, index: usize, id: &str) -> Option<(&'a Call, &'a ToolResult)> {
    // A result in a model input arrived before any invocation in its output.
    let mut observed = None;
    for call in &run.calls[index + 1..] {
        for result in call
            .tool_results
            .iter()
            .filter(|result| result.tool_call_id == id)
        {
            if result
                .outcome
                .as_ref()
                .is_some_and(|outcome| !outcome.is_null())
            {
                return Some((call, result));
            }
            observed.get_or_insert((call, result));
        }
        if call
            .decisions
            .iter()
            .any(|decision| decision.tool_call_id.as_deref() == Some(id))
        {
            break;
        }
    }
    observed
}
