//! Find earlier source edits on requests with the same assembly and decoding settings.

use nexusweave::{replay, Availability, Call, ContextItem, Run};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string()))
}

pub(super) struct Scope {
    org: String,
    user: Option<String>,
    producer: String,
}

impl Scope {
    pub fn read(attributes: &serde_json::Map<String, Value>) -> Result<Self, &'static str> {
        let identity = |key| {
            attributes
                .get(key)
                .map(|value| {
                    value
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_owned)
                        .ok_or("invalid_scope")
                })
                .transpose()
        };
        let org = identity("nexusweave.organization.id")?.ok_or("no_organization")?;
        let user = if attributes.contains_key("nexusweave.user.id") {
            identity("nexusweave.user.id")?
        } else {
            identity("user.id")?
        };
        Ok(Self {
            org,
            user,
            producer: identity("nexusweave.history.source")?.unwrap_or_else(|| "otlp".into()),
        })
    }

    pub fn lookup(&self, store: &super::store::Store, call: &Call) -> crate::Result<Value> {
        let Some(layout) = Layout::new(call) else {
            return Ok(json!({"status":"no_layout","edits":[]}));
        };
        let partition = layout.partition(&self.producer, &self.org, self.user.as_deref());
        store.prior_edits(&partition, &layout.body)
    }
}

pub(super) struct Layout {
    shape: String,
    pub body: String,
    slots: BTreeMap<String, (String, usize)>,
}

fn anchor(item: &ContextItem) -> Option<(String, usize, usize)> {
    if let Some(fragment) = &item.request_fragment {
        return Some((
            fragment.pointer.clone(),
            fragment.text_range.start_byte,
            fragment.text_range.end_byte,
        ));
    }
    let index = item.evidence.message_index?;
    Some((
        format!("/messages/{index}/content"),
        0,
        item.text.as_ref()?.len(),
    ))
}

impl Layout {
    pub fn new(call: &Call) -> Option<Self> {
        let model = call.model.as_deref()?;
        let mut request: Value =
            serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
        let body = digest(&request);
        let mut fields = BTreeMap::<String, Vec<_>>::new();
        for item in call
            .context
            .iter()
            .filter(|i| i.availability == Availability::Included && i.source_id.is_some())
        {
            let (pointer, start, end) = anchor(item)?;
            let text = request.pointer(&pointer)?.as_str()?;
            if item.source_version.as_deref()?.is_empty()
                || item.text.is_none()
                || start >= end
                || text.get(start..end) != item.text.as_deref()
                || (item.request_fragment.is_none() && text.len() != end)
            {
                return None;
            }
            fields.entry(pointer).or_default().push((start, end, item));
        }
        if fields.is_empty() {
            return None;
        }
        // All unbound request fields stay fixed except a separate final plain user question.
        let question = request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                let last = messages.last()?;
                (last["role"] == "user" && last["content"].is_string())
                    .then(|| format!("/messages/{}/content", messages.len() - 1))
            });
        let mut slots = BTreeMap::new();
        let pointers: Vec<_> = fields.keys().cloned().collect();
        for (pointer, mut fragments) in fields {
            let field = request.pointer_mut(&pointer)?;
            let text = field.as_str()?;
            let mut parts = Vec::new();
            let mut end = 0;
            fragments.sort_by_key(|(start, _, _)| *start);
            for (i, (start, next, item)) in fragments.into_iter().enumerate() {
                parts.push(json!(text.get(end..start)?));
                parts.push(json!([item.source_id, item.source_version, item.role]));
                slots.insert(item.id.clone(), (pointer.clone(), i));
                end = next;
            }
            parts.push(json!(text.get(end..)?));
            *field = json!(parts);
        }
        if let Some(pointer) = &question {
            let field = request.pointer_mut(pointer)?;
            if field.is_string() {
                *field = Value::Null;
            }
        }
        Some(Self {
            shape: digest(&json!([model, pointers, question, request])),
            body,
            slots,
        })
    }

    pub fn partition(&self, source: &str, org: &str, user: Option<&str>) -> String {
        digest(&json!([self.shape, source, org, user]))
    }

    pub fn observation(&self, run: &Run, report: &replay::Replay, id: &str) -> Option<Value> {
        let variant = report
            .variants
            .iter()
            .find(|v| v.intervention.context_id == id)?;
        let edit = &variant.intervention;
        let slot = self.slots.get(id)?;
        if report.rescored_from.is_some()
            || report.baseline_summary.matched != report.baseline_summary.attempted
            || slot.0 != edit.pointer
            || !matches!(
                edit.operation,
                replay::Operation::ReplaceText | replay::Operation::ReplaceTextFragment
            )
        {
            return None;
        }
        let setup = digest(&json!([
            slot,
            report.runner_id,
            report.recorded.kind,
            report.recorded.label,
            report.match_rule,
            report.sampling_design,
            report.baseline_summary.attempted,
            variant.summary.attempted,
            edit.operation,
            edit.replacement
        ]));
        Some(
            json!({"partition":self.partition(&run.source, run.organization_id.as_deref()?, run.user_id.as_deref()),
            "body":self.body,"setup":setup,"replacement":edit.replacement.chars().take(160).collect::<String>()}),
        )
    }
}
