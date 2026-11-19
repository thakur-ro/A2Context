//! Bind optional application declarations to evidence normalized by either trace importer.

use crate::{
    Availability, Basis, Call, ContextItem, ContextMetadata, DecisionScope, Evidence,
    RecordedRequest, Relationship, RequestFragment, Rule,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SEMANTICS: &str = "nexusweave.context.semantics";
const PREFLIGHT: &str = "nexusweave.preflight";
const CANDIDATES: &str = "nexusweave.candidate_probes";
const SHADOW: &str = "nexusweave.shadow_probes";
pub(crate) const REQUEST: &str = "nexusweave.request.body";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declarations {
    version: u32,
    #[serde(default)]
    context: Vec<Source>,
    #[serde(default)]
    request_sources: Vec<RequestSource>,
    #[serde(default)]
    decisions: Vec<Decision>,
    #[serde(default)]
    relationships: Vec<Relation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    field: String,
    message_index: Option<usize>,
    part_index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    anchor: Anchor,
    source_id: String,
    source_version: Option<String>,
    authority: Option<String>,
    rule: Option<Rule>,
    request_fragment: Option<RequestFragment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestSource {
    id: String,
    source_id: String,
    source_version: Option<String>,
    kind: Option<String>,
    role: Option<String>,
    authority: Option<String>,
    rule: Option<Rule>,
    request_fragment: RequestFragment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    anchor: Anchor,
    applies_to: DecisionScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Relation {
    context: ContextAnchor,
    decision: Anchor,
    relation: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ContextAnchor {
    Recorded(Anchor),
    RequestSource(RequestSourceAnchor),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestSourceAnchor {
    request_source_id: String,
}

pub(crate) fn apply(call: &mut Call, metadata: &Value, field: impl Fn(&str) -> String) {
    let span = call.span_id.clone();
    let evidence = |key: &str| Evidence {
        span_id: span.clone(),
        field: field(key),
        message_index: None,
        part_index: None,
    };
    if let Some(body) = metadata.get(REQUEST).and_then(Value::as_str) {
        let captured = RecordedRequest {
            body: body.to_owned(),
            evidence: evidence(REQUEST),
        };
        // A truncated capture still leaves the trace usable.
        if captured.parse().is_ok() {
            call.recorded_request = Some(captured);
        }
    }
    if let Some(value) = metadata.get(SEMANTICS) {
        let evidence = evidence(SEMANTICS);
        let parsed = match value {
            Value::String(value) => serde_json::from_str::<Declarations>(value),
            value => Declarations::deserialize(value),
        };
        let issue = parsed
            .map_err(|error| format!("Invalid context declaration. {error}"))
            .and_then(|declarations| declarations.bind(call, &evidence))
            .err();
        call.context_metadata = Some(ContextMetadata { evidence, issue });
    }
    if let Some(value) = metadata.get(PREFLIGHT) {
        let receipt = match value {
            Value::String(value) => serde_json::from_str(value),
            value => crate::preflight::Receipt::deserialize(value),
        };
        call.preflight_receipt = Some(crate::preflight::CapturedReceipt {
            evidence: evidence(PREFLIGHT),
            receipt: receipt.ok(),
        });
    }
    if let Some(value) = metadata.get(CANDIDATES) {
        let reports = match value {
            Value::String(value) => serde_json::from_str(value),
            value => Vec::<crate::replay::candidates::Report>::deserialize(value),
        };
        call.candidate_probes = Some(crate::replay::candidates::CapturedReports {
            evidence: evidence(CANDIDATES),
            reports: reports.ok(),
        });
    }
    if let Some(value) = metadata.get(SHADOW) {
        let reports = match value {
            Value::String(value) => serde_json::from_str(value),
            value => Vec::<crate::replay::shadow::Report>::deserialize(value),
        };
        call.shadow_probes = Some(crate::replay::shadow::CapturedReports {
            evidence: evidence(SHADOW),
            reports: reports.ok(),
        });
    }
}

impl Declarations {
    fn bind(self, call: &mut Call, evidence: &Evidence) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "Unsupported context declaration version {}",
                self.version
            ));
        }
        let context = index(call.context.iter().map(|item| &item.evidence));
        let decisions = index(call.decisions.iter().map(|item| &item.evidence));
        let mut sources = BTreeMap::new();
        for source in self.context {
            let slot = source.anchor.resolve(&context, "Context")?;
            check_source(
                &source.source_id,
                source.source_version.as_deref(),
                source.rule.as_ref(),
            )?;
            if let Some(rule) = &source.rule {
                if rule.value.is_some() && call.context[slot].availability != Availability::Included
                {
                    return Err("A rule value cannot be attached to unavailable context".into());
                }
            }
            if let Some(fragment) = &source.request_fragment {
                fragment.validate()?;
            }
            if sources.insert(slot, source).is_some() {
                return Err("Multiple source declarations target the same context item".into());
            }
        }
        let request_sources = project_request_sources(self.request_sources, call)?;
        let request_slots: BTreeMap<_, _> = request_sources
            .keys()
            .enumerate()
            .map(|(index, id)| (id.as_str(), call.context.len() + index))
            .collect();
        let mut scopes = BTreeMap::new();
        for decision in self.decisions {
            let slot = decision.anchor.resolve(&decisions, "Decision")?;
            check_scope(&decision.applies_to.key, &decision.applies_to.scope)?;
            if scopes.insert(slot, decision.applies_to).is_some() {
                return Err("Multiple scope declarations target the same decision".into());
            }
        }
        let mut relations = BTreeSet::new();
        for relation in self.relationships {
            let context_id = match relation.context {
                ContextAnchor::Recorded(anchor) => anchor.resolve(&context, "Context")?,
                ContextAnchor::RequestSource(anchor) => *request_slots
                    .get(anchor.request_source_id.as_str())
                    .ok_or_else(|| {
                        format!("Unknown request source ID {}", anchor.request_source_id)
                    })?,
            };
            let decision_id = relation.decision.resolve(&decisions, "Decision")?;
            if relation.relation.trim().is_empty() {
                return Err("A declared relationship needs a name".into());
            }
            relations.insert((context_id, decision_id, relation.relation));
        }

        for (slot, source) in sources {
            let item = &mut call.context[slot];
            if item.kind == "external_context"
                && item.content_reference.is_none()
                && item.source_id.as_deref() != Some(source.source_id.as_str())
            {
                item.content_reference = item.source_id.take();
            }
            item.source_id = Some(source.source_id);
            item.source_version = source.source_version;
            item.authority = source.authority;
            item.rule = source.rule;
            item.request_fragment = source.request_fragment;
        }
        for (slot, scope) in scopes {
            call.decisions[slot].applies_to = Some(scope);
        }
        call.context.extend(request_sources.into_values());
        call.relationships
            .extend(
                relations
                    .into_iter()
                    .map(|(context, decision, relation)| Relationship {
                        decision_id: call.decisions[decision].id.clone(),
                        context_id: call.context[context].id.clone(),
                        relation,
                        basis: Basis::Declared,
                        evidence: Some(evidence.clone()),
                        excerpt: None,
                    }),
            );
        Ok(())
    }
}

fn project_request_sources(
    sources: Vec<RequestSource>,
    call: &Call,
) -> Result<BTreeMap<String, ContextItem>, String> {
    if sources.is_empty() {
        return Ok(BTreeMap::new());
    }
    let captured = call
        .recorded_request
        .as_ref()
        .ok_or("Request sources require a captured request body")?;
    let request = captured.parse()?;
    let existing: BTreeSet<_> = call.context.iter().map(|item| item.id.as_str()).collect();
    let mut items = BTreeMap::new();
    for source in sources {
        check_source(
            &source.source_id,
            source.source_version.as_deref(),
            source.rule.as_ref(),
        )?;
        if source.id.trim().is_empty()
            || source
                .kind
                .as_deref()
                .is_some_and(|kind| kind.trim().is_empty())
            || source
                .role
                .as_deref()
                .is_some_and(|role| role.trim().is_empty())
        {
            return Err(
                "Request source IDs, kinds, and roles require nonempty values when supplied".into(),
            );
        }
        let id = format!("request-source:{}", source.id);
        if existing.contains(id.as_str()) || items.contains_key(&source.id) {
            return Err(format!("Duplicate request source ID {}", source.id));
        }
        source
            .request_fragment
            .validate()
            .map_err(|error| format!("Request source {}: {error}", source.id))?;
        let text = source.request_fragment.text(&request).ok_or_else(|| {
            format!(
                "Request source {} does not identify a UTF-8 text range at {}",
                source.id, source.request_fragment.pointer
            )
        })?;
        items.insert(
            source.id,
            ContextItem {
                id,
                kind: source.kind.unwrap_or_else(|| "request_fragment".into()),
                role: source.role,
                text: Some(text.into()),
                content_reference: None,
                document_identity: None,
                request_fragment: Some(source.request_fragment),
                source_id: Some(source.source_id),
                source_version: source.source_version,
                authority: source.authority,
                availability: Availability::Included,
                evidence: captured.evidence.clone(),
                rule: source.rule,
            },
        );
    }
    Ok(items)
}

type Slots<'a> = BTreeMap<(&'a str, Option<usize>, Option<usize>), Option<usize>>;

fn index<'a>(items: impl Iterator<Item = &'a Evidence>) -> Slots<'a> {
    let mut slots = Slots::new();
    for (slot, anchor) in items.enumerate() {
        slots
            .entry((&anchor.field, anchor.message_index, anchor.part_index))
            .and_modify(|value| *value = None)
            .or_insert(Some(slot));
    }
    slots
}

impl Anchor {
    fn resolve(&self, slots: &Slots<'_>, kind: &str) -> Result<usize, String> {
        let key = (self.field.as_str(), self.message_index, self.part_index);
        slots.get(&key).copied().flatten().ok_or_else(|| {
            let mut path = self.field.clone();
            if let Some(index) = self.message_index {
                path.push_str(&format!(" / message index {index}"));
            }
            if let Some(index) = self.part_index {
                path.push_str(&format!(" / part index {index}"));
            }
            format!("{kind} anchor does not identify one recorded item at {path}")
        })
    }
}

fn check_scope(key: &str, scope: &str) -> Result<(), String> {
    if key.trim().is_empty() || scope.trim().is_empty() {
        return Err("Rule and decision scopes require a nonempty key and scope".into());
    }
    Ok(())
}

fn check_source(id: &str, version: Option<&str>, rule: Option<&Rule>) -> Result<(), String> {
    if id.trim().is_empty() || version.is_some_and(|value| value.trim().is_empty()) {
        return Err("Source IDs and versions require nonempty values".into());
    }
    if let Some(rule) = rule {
        check_scope(&rule.key, &rule.scope)?;
    }
    Ok(())
}
