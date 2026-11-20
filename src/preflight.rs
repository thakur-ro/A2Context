//! Inspect declared context rules before sending a request to a model.
//!
//! Preflight checks the supplied request and source declarations without a
//! model call. Each finding includes the sources and their request anchors.
//! The [workflow guide](https://github.com/thakur-ro/NexusWeave/blob/main/docs/guide.md#check-context-before-inference)
//! walks through the same check from the CLI and HTTP API.

use crate::{
    genai, span_metadata, Call, ContextItem, DecisionScope, Evidence, FindingKind, RequestFragment,
    Rule,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const VERSION: u32 = 1;

mod anchor_overlap;
mod required_sources;
pub use anchor_overlap::{AnchorOverlap, NamedRange, SharedRange};
pub(crate) use required_sources::RequiredSources;
pub use required_sources::{Alternative, SourceCoverage};

/// The context about to be sent for one inference request.
#[derive(Clone, Debug, Deserialize, Serialize)]
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
    /// Present when the caller asked for a specific rule. Counts captured declarations only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_coverage: Option<ScopeCoverage>,
    /// Coverage of caller-specified alternatives in this pending request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_coverage: Option<SourceCoverage>,
    /// Distinct source IDs covering the same bytes in a captured string field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_overlap: Option<AnchorOverlap>,
    pub findings: Vec<Finding>,
    /// Attach to the resulting inference span as `nexusweave.preflight`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

/// Capture coverage for a rule that the caller explicitly asked to inspect.
#[derive(Debug, Serialize)]
pub struct ScopeCoverage {
    pub scope: DecisionScope,
    pub declarations: usize,
    pub known_values: usize,
}

/// Counts declarations and supplied values for the requested key and scope.
/// Zero declarations means that the capture supplied no matching rule.
pub fn scope_coverage(call: &Call, scope: &DecisionScope) -> ScopeCoverage {
    let mut declarations = 0;
    let mut known_values = 0;
    for (_, rule) in rules(call, Some(scope)) {
        declarations += 1;
        known_values += usize::from(rule.value.is_some());
    }
    ScopeCoverage {
        scope: scope.clone(),
        declarations,
        known_values,
    }
}

/// The scope and fingerprint of a check on the exact captured request body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub scope: Option<DecisionScope>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    required_sources: Option<RequiredSources>,
}

/// A preflight receipt recorded on an inference call.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CapturedReceipt {
    pub evidence: Evidence,
    pub receipt: Option<Receipt>,
}

#[derive(Debug, Serialize)]
pub struct RecordedCheck<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub evidence: &'a Evidence,
    pub scope: Option<&'a DecisionScope>,
    #[serde(flatten)]
    pub verification: Verification,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verification {
    Matched { report: Box<Report> },
    Unverified { issue: &'static str },
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
pub fn check(mut request: Request) -> Result<Report, String> {
    let required = RequiredSources::take(&mut request.attributes)?;
    let (call, _, scope) = pending_call(request)?;
    Ok(report_for_pending(&call, scope, required))
}

pub(crate) fn report_for_pending(
    call: &Call,
    scope: Option<DecisionScope>,
    required: Option<RequiredSources>,
) -> Report {
    let mut report = compare(call, scope.as_ref());
    report.source_coverage = required.as_ref().map(|r| r.evaluate(call));
    report.anchor_overlap = anchor_overlap::measure(call);
    if let Some(fingerprint) = receipt_fingerprint(call, scope.as_ref(), required.as_ref()) {
        report.receipt = Some(Receipt {
            version: if required.is_some() { 2 } else { VERSION },
            scope,
            fingerprint,
            required_sources: required,
        });
    }
    report
}

pub(crate) fn pending_call(
    request: Request,
) -> Result<(Call, Option<Value>, Option<DecisionScope>), String> {
    let Request {
        version,
        scope,
        attributes,
    } = request;
    if version != VERSION {
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
    if let Some(issue) = call
        .context_metadata
        .as_ref()
        .and_then(|metadata| metadata.issue.as_ref())
    {
        return Err(issue.clone());
    }
    let body = call
        .recorded_request
        .as_ref()
        .map(crate::RecordedRequest::parse)
        .transpose()?;
    if attributes.get(span_metadata::REQUEST).is_some() && body.is_none() {
        return Err("The captured request body must be a JSON object encoded as a string".into());
    }

    for item in &call.context {
        if let Some(fragment) = &item.request_fragment {
            let selected = body.as_ref().and_then(|body| fragment.text(body));
            if selected.is_none() || selected != item.text.as_deref() {
                return Err(format!(
                    "Request fragment for {} does not match the captured body",
                    item.id
                ));
            }
        }
    }
    Ok((call, body, scope))
}

pub(crate) fn recorded<'a>(run_id: &'a str, call: &'a Call) -> Option<RecordedCheck<'a>> {
    let captured = call.preflight_receipt.as_ref()?;
    let scope = captured
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.scope.as_ref());
    let verification = match verify(call, captured) {
        Ok(receipt) => {
            let mut report = compare(call, scope);
            report.source_coverage = receipt.required_sources.as_ref().map(|r| r.evaluate(call));
            if receipt.version == 2 {
                report.anchor_overlap = anchor_overlap::measure(call);
            }
            Verification::Matched {
                report: Box::new(report),
            }
        }
        Err(issue) => Verification::Unverified { issue },
    };
    Some(RecordedCheck {
        run_id,
        call_id: &call.id,
        evidence: &captured.evidence,
        scope,
        verification,
    })
}

fn verify<'a>(call: &Call, captured: &'a CapturedReceipt) -> Result<&'a Receipt, &'static str> {
    let receipt = captured
        .receipt
        .as_ref()
        .ok_or("The workflow supplied an unreadable preflight receipt.")?;
    if !matches!(
        (receipt.version, &receipt.required_sources),
        (1, None) | (2, Some(_))
    ) {
        return Err("The preflight receipt version is unsupported.");
    }
    if call
        .context_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.issue.is_some())
    {
        return Err("The recorded source declarations could not be read.");
    }
    if receipt
        .required_sources
        .as_ref()
        .is_some_and(|r| r.validate().is_err())
    {
        return Err("The receipt contains an invalid source requirement.");
    }
    let actual = receipt_fingerprint(
        call,
        receipt.scope.as_ref(),
        receipt.required_sources.as_ref(),
    )
    .ok_or("The original request body was unavailable for verification.")?;
    if actual != receipt.fingerprint {
        if receipt.version == 2 {
            return Err("The request, source requirement or captured anchors differ from the preflight check.");
        }
        return Err(
            "The recorded request or its declared rule sources differ from the preflight check.",
        );
    }
    Ok(receipt)
}

pub(crate) fn verified_receipt(call: &Call) -> Option<&Receipt> {
    verify(call, call.preflight_receipt.as_ref()?).ok()
}

/// Recompute capture coverage only when a version-two receipt matches the recorded inputs.
pub fn verified_source_coverage(call: &Call) -> Option<SourceCoverage> {
    let captured = call.preflight_receipt.as_ref()?;
    let required = captured.receipt.as_ref()?.required_sources.as_ref()?;
    verify(call, captured).ok()?;
    Some(required.evaluate(call))
}

fn receipt_fingerprint(
    call: &Call,
    scope: Option<&DecisionScope>,
    required: Option<&RequiredSources>,
) -> Option<String> {
    let base = fingerprint(call, scope)?;
    let Some(required) = required else {
        return Some(base);
    };
    let mut anchors: Vec<_> = call
        .context
        .iter()
        .filter(|item| item.request_fragment.is_some() && item.source_id.is_some())
        .map(|item| {
            crate::canonical_json(&json!([
                item.source_id,
                item.source_version,
                item.role,
                item.request_fragment,
                item.availability,
                item.text
            ]))
        })
        .collect();
    anchors.sort_unstable();
    let payload = json!(["nexusweave.preflight", 2, base, required, anchors]);
    Some(crate::replay::sha256(payload.to_string().as_bytes()))
}

fn fingerprint(call: &Call, scope: Option<&DecisionScope>) -> Option<String> {
    let body = &call.recorded_request.as_ref()?.body;
    let mut manifest: Vec<_> = rules(call, scope)
        .map(|(item, rule)| {
            crate::canonical_json(&json!([
                item.source_id,
                item.source_version,
                item.authority,
                item.availability,
                item.request_fragment,
                item.text,
                rule,
            ]))
        })
        .collect();
    manifest.sort_unstable();
    // Trace adapters can assign different context IDs and evidence anchors.
    let body_hash = crate::replay::sha256(body.as_bytes());
    let payload = json!(["nexusweave.preflight", VERSION, body_hash, scope, manifest]);
    Some(crate::replay::sha256(payload.to_string().as_bytes()))
}

pub(crate) fn rules<'a>(
    call: &'a Call,
    scope: Option<&'a DecisionScope>,
) -> impl Iterator<Item = (&'a ContextItem, &'a Rule)> {
    call.context.iter().filter_map(move |item| {
        let rule = item.rule.as_ref()?;
        scope
            .is_none_or(|scope| (&rule.key, &rule.scope) == (&scope.key, &scope.scope))
            .then_some((item, rule))
    })
}

fn compare(call: &Call, scope: Option<&DecisionScope>) -> Report {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for (item, rule) in rules(call, scope) {
        groups
            .entry((rule.key.clone(), rule.scope.clone()))
            .or_default()
            .push(Source {
                context_id: item.id.clone(),
                source_id: item.source_id.clone(),
                source_version: item.source_version.clone(),
                authority: item.authority.clone(),
                value: rule.value.clone(),
                evidence: item.evidence.clone(),
                request_fragment: item.request_fragment.clone(),
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
    Report {
        version: VERSION,
        model: call.model.clone(),
        captured_items: call.context.len(),
        scoped_rule_items,
        scope_coverage: scope.map(|s| scope_coverage(call, s)),
        source_coverage: None,
        anchor_overlap: None,
        findings,
        receipt: None,
    }
}
