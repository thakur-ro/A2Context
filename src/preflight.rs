//! Inspect declared context rules before sending a request to a model.
//!
//! Preflight checks the supplied request and source declarations without a
//! model call. Each finding includes the sources and their request anchors.
//! The [workflow guide](https://github.com/thakur-ro/NexusWeave/blob/main/docs/guide.md#check-context-before-inference)
//! walks through the same check from the CLI and HTTP API.

use crate::{genai, span_metadata, DecisionScope, Evidence, FindingKind, RequestFragment};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The context about to be sent for one inference request.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Preflight format version. The current version is `1`.
    pub version: u32,
    /// Optional rule key and scope to check. Omit it to check every declared scope.
    pub scope: Option<DecisionScope>,
    /// OpenTelemetry GenAI input attributes and NexusWeave source declarations.
    /// `nexusweave.request.body` can carry a JSON object encoded as a string.
    pub attributes: Map<String, Value>,
}

/// Findings from the supplied input. Source text and the request body stay out of the report.
#[derive(Debug, Serialize)]
pub struct Report {
    pub version: u32,
    pub model: Option<String>,
    pub captured_items: usize,
    pub scoped_rule_items: usize,
    pub findings: Vec<Finding>,
}

/// A comparison of two or more declarations for the same rule key and scope.
#[derive(Debug, Serialize)]
pub struct Finding {
    /// Conflicts take precedence over missing values, followed by duplicate values.
    pub kind: FindingKind,
    pub key: String,
    pub scope: String,
    pub sources: Vec<Source>,
}

/// A rule declaration and its location in the captured request.
#[derive(Debug, Serialize)]
pub struct Source {
    pub context_id: String,
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub authority: Option<String>,
    pub value: Option<Value>,
    pub evidence: Evidence,
    pub request_fragment: Option<RequestFragment>,
}

/// Compare the declared rule values in a pending request.
///
/// Values are grouped by rule key and scope, then compared for JSON equality.
/// A conflict means at least two known values disagree. With no disagreement,
/// a missing value gives `InsufficientEvidence`. An entirely matching group
/// of known values gives `Duplicate`. Singletons produce no finding.
/// The result describes input conditions and carries no model attribution.
///
/// # Example
///
/// ```
/// use nexusweave::{preflight, FindingKind};
///
/// let input = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/preflight-returns.json"));
/// let report = preflight::check(serde_json::from_str(input)?)?;
/// assert_eq!(report.findings[0].kind, FindingKind::Conflict);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Rejects unsupported versions, output attributes, invalid declarations, and
/// request fragments that do not match the captured body.
pub fn check(request: Request) -> Result<Report, String> {
    let Request {
        version,
        scope,
        attributes,
    } = request;
    if version != 1 {
        return Err(format!("Unsupported preflight version {version}"));
    }
    if scope
        .as_ref()
        .is_some_and(|scope| scope.key.trim().is_empty() || scope.scope.trim().is_empty())
    {
        return Err("Preflight scope requires a nonempty key and scope".into());
    }
    if attributes.contains_key("gen_ai.output.messages") {
        return Err("Preflight accepts inference inputs only".into());
    }

    let mut call = genai::empty_call("preflight", None, None);
    call.span_id = None;
    let attributes = Value::Object(attributes);
    let mut call = genai::record_inference(call, &attributes)?;
    span_metadata::apply(&mut call, &attributes, str::to_owned);
    if let Some(issue) = call.context_metadata.and_then(|metadata| metadata.issue) {
        return Err(issue);
    }
    let body = call
        .recorded_request
        .as_ref()
        .map(crate::RecordedRequest::parse)
        .transpose()?;
    if attributes.get(span_metadata::REQUEST).is_some() && body.is_none() {
        return Err("The captured request body must be a JSON object encoded as a string".into());
    }

    let captured_items = call.context.len();
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for item in call.context {
        if let Some(fragment) = &item.request_fragment {
            let selected = body.as_ref().and_then(|body| fragment.text(body));
            if selected.is_none() || selected != item.text.as_deref() {
                return Err(format!(
                    "Request fragment for {} does not match the captured body",
                    item.id
                ));
            }
        }
        let Some(rule) = item.rule else { continue };
        if scope
            .as_ref()
            .is_some_and(|scope| (&rule.key, &rule.scope) != (&scope.key, &scope.scope))
        {
            continue;
        }
        groups
            .entry((rule.key, rule.scope))
            .or_default()
            .push(Source {
                context_id: item.id,
                source_id: item.source_id,
                source_version: item.source_version,
                authority: item.authority,
                value: rule.value,
                evidence: item.evidence,
                request_fragment: item.request_fragment,
            });
    }
    let scoped_rule_items = groups.values().map(Vec::len).sum();
    let findings = groups
        .into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|((key, scope), sources)| {
            let mut known = sources.iter().filter_map(|source| source.value.as_ref());
            let conflict = known
                .next()
                .is_some_and(|first| known.any(|value| value != first));
            let kind = if conflict {
                FindingKind::Conflict
            } else if sources.iter().any(|source| source.value.is_none()) {
                FindingKind::InsufficientEvidence
            } else {
                FindingKind::Duplicate
            };
            Finding {
                kind,
                key,
                scope,
                sources,
            }
        })
        .collect();
    Ok(Report {
        version,
        model: call.model,
        captured_items,
        scoped_rule_items,
        findings,
    })
}
