//! Find earlier source edits on requests with the same assembly and decoding settings.

use nexusweave::{replay, Availability, Call, ContextItem, Run};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// A request body votes once for each compatible edit. Missing pairs cannot establish equality.
pub(super) const PRIOR_EDITS: &str = "WITH recent AS MATERIALIZED (
    SELECT detail,trace_id,call_id,group_id,observed,n FROM observations
    WHERE json_extract(detail,'$.prior.partition')=?1
    ORDER BY observed DESC,trace_id,n LIMIT 513
), tested AS (
    SELECT * FROM recent ORDER BY observed DESC,trace_id,n LIMIT 512
), trials AS (
    SELECT json_extract(detail,'$.prior.setup') AS setup,json_extract(detail,'$.prior.body') AS body,
        json_extract(detail,'$.effect_pp')<0 AS lower,json_extract(detail,'$.response_pair.changed') AS changed,
        json_object('trace',trace_id,'call',call_id,'group',group_id,'factor',json_extract(detail,'$.factor'),
            'source',json_extract(detail,'$.sources[0].source'),'version',json_extract(detail,'$.sources[0].version'),
            'replacement',json_extract(detail,'$.prior.replacement'),'runner',json_extract(detail,'$.runner'),
            'rule',json_extract(detail,'$.match_rule'),'outcome',json_extract(detail,'$.outcome'),
            'report',json_extract(detail,'$.measurement.replay_id'),'decision',json_extract(detail,'$.measurement.location.decision_id'),
            'context',json_extract(detail,'$.measurement.context_id')) AS example
    FROM tested WHERE json_extract(detail,'$.prior.body')!=?2
), by_body AS (
    SELECT setup,body,min(lower) AS lo,max(lower) AS hi,
        CASE WHEN min(changed)!=max(changed) THEN 'mixed'
             WHEN count(changed)<count(*) THEN 'unresolved'
             WHEN min(changed)=1 THEN 'changed' ELSE 'held' END AS proposal,
        min(example) AS example,min(example) FILTER (WHERE lower) AS lower_example
    FROM trials GROUP BY setup,body
), ranked AS (
    SELECT setup,count(*) AS bodies,sum(lo) AS lower,sum(1-hi) AS equal,sum(hi-lo) AS mixed,
        sum(proposal='changed') AS changed,sum(proposal='held') AS held,
        sum(proposal='mixed') AS disputed,sum(proposal='unresolved') AS unresolved,
        coalesce(min(example) FILTER (WHERE proposal='changed'),
            min(lower_example) FILTER (WHERE lo=1),min(example)) AS example
    FROM by_body GROUP BY setup HAVING count(*)>=2
    ORDER BY changed DESC,CASE WHEN sum(proposal='changed')>0 THEN sum(proposal IN ('mixed','unresolved')) END ASC,
        lower DESC,mixed ASC,bodies DESC,setup LIMIT 5
) SELECT json_object('status','checked','truncated',(SELECT count(*)>512 FROM recent),
    'edits',json_group_array(json_object('setup',setup,'requests',bodies,'lower',lower,'equal',equal,'mixed',mixed,
        'proposals',json_object('changed',changed,'held',held,'mixed',disputed,'unresolved',unresolved),
        'example',json(example)))) FROM ranked";

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
        let mut history = store.prior_edits(&partition, &layout.body)?;
        let top = history["edits"]
            .as_array()
            .and_then(|rows| rows.first())
            .filter(|row| {
                row["proposals"]["changed"].as_u64().unwrap_or(0) > 0
                    || row["lower"].as_u64().unwrap_or(0) > 0
            });
        if let Some(top) = top {
            let report = store.prior_replay(&top["example"])?;
            history["experiment"] = json!(report.and_then(|r| layout.prepare(call, top, &r)));
        }
        Ok(history)
    }
}

pub(super) struct Layout {
    shape: String,
    pub body: String,
    slots: BTreeMap<String, (String, usize)>,
}

/// Join only measurements verified on this pending call and the exact prepared edit.
pub(super) fn current_tests(
    experiment: &mut Value,
    call: &Call,
    verified: &[replay::shadow::RecordedProbe],
    prefill: Option<&Value>,
) {
    let id = &experiment["interventions"][0]["context_id"];
    let digest = &experiment["edited_request_sha256"];
    let score = prefill
        .and_then(|p| p["ranked"].as_array())
        .and_then(|rows| {
            rows.iter()
                .find(|r| r["context_id"] == *id && r["request_sha256"] == *digest)
        });
    let reports = call
        .shadow_probes
        .as_ref()
        .and_then(|p| p.reports.as_deref())
        .unwrap_or_default();
    let mut tests = Vec::new();
    let mut total = 0;
    for (probe, report) in verified.iter().zip(reports) {
        let replay::shadow::Verification::Matched { measurement } = &probe.verification else {
            continue;
        };
        for (resolved, effect) in measurement.effects.iter().zip(&report.effects) {
            if resolved.context_id != id || effect.request_sha256 != *digest {
                continue;
            }
            total += 1;
            if tests.len() < 5 {
                tests.push(json!({"report":report.id,"runner":report.runner_id,
                    "baseline":report.baseline_summary,"edited":effect.result.summary,
                    "pair":super::outcomes::compare(&report.baseline, &effect.result.trials)}));
            }
        }
    }
    experiment["first_token_js_bits"] = json!(score.and_then(|r| r["js_bits"].as_f64()));
    experiment["proposal_tests"] = json!(tests);
    experiment["total_proposal_tests"] = json!(total);
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
        let setup = Self::setup(slot, report, variant);
        Some(
            json!({"partition":self.partition(&run.source, run.organization_id.as_deref()?, run.user_id.as_deref()),
            "body":self.body,"setup":setup,"replacement":edit.replacement.chars().take(160).collect::<String>()}),
        )
    }

    fn setup(slot: &(String, usize), report: &replay::Replay, variant: &replay::Variant) -> String {
        let edit = &variant.intervention;
        digest(&json!([
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
        ]))
    }

    fn prepare(&self, call: &Call, top: &Value, report: &replay::Replay) -> Option<Value> {
        let example = &top["example"];
        let variant = report
            .variants
            .iter()
            .find(|v| v.intervention.context_id == example["context"])?;
        let item = call.context.iter().find(|item| {
            item.source_id.as_deref() == example["source"].as_str()
                && item.source_version.as_deref() == example["version"].as_str()
                && self.slots.get(&item.id).is_some_and(|slot| {
                    slot.0 == variant.intervention.pointer
                        && Self::setup(slot, report, variant) == top["setup"]
                })
        })?;
        let edit = replay::Intervention {
            context_id: item.id.clone(),
            text_range: item.request_fragment.as_ref().map(|f| f.text_range),
            ..variant.intervention.clone()
        };
        if item.text.as_deref() == Some(edit.replacement.as_str()) {
            return Some(json!({"status":"already_applied","baseline_request_sha256":self.body}));
        }
        let (original, edited) = replay::prefill::edit_digests(call, &edit).ok()?;
        Some(
            json!({"status":"ready","interventions":[edit],"baseline_request_sha256":original,"edited_request_sha256":edited}),
        )
    }
}
