//! Rank named source edits by an adapter's first-token Jensen–Shannon divergences.
//! The adapter sees all request bodies. Verification binds its reported scalars to
//! the captured inputs without loading a model or generating a continuation.

use super::{prepare_edit, recorded_context, sha256, Intervention};
use crate::{preflight, Call};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub mod routing;

pub const ATTRIBUTE: &str = "nexusweave.prefill.probe";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scores {
    pub version: u32,
    pub model_id: String,
    pub model_revision: String,
    pub runtime: String,
    pub tokenizer_sha256: String,
    pub baseline_request_sha256: String,
    /// Highest-probability token before decoding, under the reported tokenizer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_token_id: Option<u32>,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub context_id: String,
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub request_sha256: String,
    pub js_bits: f64,
    /// Supply for every edit when the baseline includes an opening token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_token_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub interventions: Vec<Intervention>,
    scores: Scores,
    /// Optional evaluation metadata. It is never sent to the scoring adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_reference: Option<Value>,
    /// Optional token readout from an instrumented greedy rerun.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_contrast: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing_test: Option<Value>,
}

fn validate_edits(call: &Call, body: &Value, edits: &[Intervention]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if edits.is_empty() || edits.len() > 32 || !edits.iter().all(|e| seen.insert(&e.context_id)) {
        return Err("Choose 1 to 32 distinct sources for a first-token test.".into());
    }
    let matches = super::request_field_matches(call, body)?;
    for edit in edits {
        let fields = matches
            .iter()
            .find(|m| m.context_id == edit.context_id)
            .map(|m| m.fields.as_slice());
        if !matches!(fields, Some([field]) if field.pointer == edit.pointer && field.text_range == edit.text_range)
        {
            return Err(format!(
                "Source {} requires one verified location in the captured request.",
                edit.context_id
            ));
        }
    }
    Ok(())
}

fn body_with_edit(call: &Call, body: &Value, edit: &Intervention) -> Result<Value, String> {
    let mut amended = body.clone();
    prepare_edit(call, edit)?.apply(&mut amended)?;
    Ok(amended)
}

fn digest(body: &Value) -> String {
    sha256(body.to_string().as_bytes())
}

fn excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let mut short: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}

impl Probe {
    fn checked(&self, call: &Call, body: &Value, excerpts: bool) -> Result<Value, String> {
        let s = &self.scores;
        validate_edits(call, body, &self.interventions)?;
        if s.version != 1
            || s.model_id.trim().is_empty()
            || s.model_revision.trim().is_empty()
            || s.runtime.trim().is_empty()
            || s.tokenizer_sha256.len() != 64
            || !s.tokenizer_sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || call
                .model
                .as_ref()
                .is_some_and(|model| model != &s.model_id)
            || body
                .get("model")
                .and_then(Value::as_str)
                .is_some_and(|model| model != s.model_id)
        {
            return Err(
                "First-token test has an invalid runtime identity or model mismatch.".into(),
            );
        }
        if s.baseline_request_sha256 != digest(body) || s.effects.len() != self.interventions.len()
        {
            return Err(
                "First-token test differs from the captured baseline or edit count.".into(),
            );
        }
        let mut ranked = Vec::new();
        for (edit, effect) in self.interventions.iter().zip(&s.effects) {
            let changed = body_with_edit(call, body, edit)?;
            let (source, before) = recorded_context(call, &edit.context_id)?;
            if edit.context_id != effect.context_id
                || source.source_id != effect.source_id
                || source.source_version != effect.source_version
                || digest(&changed) != effect.request_sha256
                || !(0.0..=1.0).contains(&effect.js_bits)
            {
                return Err(
                    "First-token score, source, or edited request digest is invalid.".into(),
                );
            }
            let mut row = json!({"context_id":source.id, "source":source.source_id, "version":source.source_version,
                "request_sha256":effect.request_sha256,"pointer":edit.pointer,"text_range":edit.text_range,
                "operation":edit.operation,"js_bits":effect.js_bits});
            match (s.top_token_id, effect.top_token_id) {
                (Some(original), Some(edited)) => {
                    if s.baseline_request_sha256 == effect.request_sha256 && original != edited {
                        return Err("Identical requests have conflicting opening token IDs.".into());
                    }
                    row["opening_changed"] = json!(original != edited);
                    if excerpts {
                        row["top_token_id"] = json!(edited);
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(
                        "An opening-token comparison needs IDs for the baseline and every edit."
                            .into(),
                    )
                }
            }
            if excerpts {
                row["before"] = json!(excerpt(before));
                row["replacement"] = json!(excerpt(&edit.replacement));
            }
            ranked.push((effect.js_bits, row));
        }
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut view = json!({"metric":"first_token_js_bits","model_id":s.model_id,"model_revision":s.model_revision,
            "runtime":s.runtime,"tokenizer_sha256":s.tokenizer_sha256,"baseline_request_sha256":s.baseline_request_sha256,
            "ranked":ranked.into_iter().map(|(_,row)| row).collect::<Vec<_>>()});
        if excerpts {
            if let Some(id) = s.top_token_id {
                view["top_token_id"] = json!(id);
            }
        }
        if let Some(raw) = &self.routing_test {
            match routing::checked(raw, self, call, body, excerpts) {
                Ok(result) => view["routing_test"] = result,
                Err(issue) => view["routing_issue"] = json!(issue),
            }
        }
        Ok(view)
    }
}

/// Verify one first-token probe edit against the recorded call and fingerprint both requests.
pub fn edit_digests(call: &Call, edit: &Intervention) -> Result<(String, String), String> {
    let captured = call
        .recorded_request
        .as_ref()
        .ok_or("Missing captured request.")?;
    let body: Value = serde_json::from_str(&captured.body).map_err(|e| e.to_string())?;
    validate_edits(call, &body, std::slice::from_ref(edit))?;
    let changed = body_with_edit(call, &body, edit)?;
    Ok((digest(&body), digest(&changed)))
}

/// Prepare all named single edits, then ask one explicit adapter to score the batch.
/// The returned request can be opened directly in the sidecar's pre-inference view.
pub fn measure(
    mut input: preflight::Request,
    interventions: Vec<Intervention>,
    scorer: impl FnOnce(&Value) -> Result<Scores, String>,
) -> Result<preflight::Request, String> {
    let (call, body, _) = preflight::pending_call(input.clone())?;
    let body = body.ok_or("First-token scoring requires the captured request body.")?;
    validate_edits(&call, &body, &interventions)?;
    let mut conditions = Vec::with_capacity(interventions.len());
    for edit in &interventions {
        let changed = body_with_edit(&call, &body, edit)?;
        let (source, _) = recorded_context(&call, &edit.context_id)?;
        conditions.push(json!({"context_id":edit.context_id,"source_id":source.source_id,"source_version":source.source_version,
            "request_sha256":digest(&changed),"request":changed}));
    }
    let batch = json!({"version":1,"baseline_request_sha256":digest(&body),"baseline":body,"edits":conditions});
    let scores = scorer(&batch)?;
    let probe = Probe {
        interventions,
        scores,
        answer_reference: None,
        token_contrast: None,
        routing_test: None,
    };
    probe.checked(&call, &body, false)?;
    input.attributes.insert(
        ATTRIBUTE.into(),
        serde_json::to_value(probe).map_err(|e| e.to_string())?,
    );
    Ok(input)
}

pub(crate) fn read(
    raw: Option<Value>,
    call: &Call,
    body: Option<&Value>,
    excerpts: bool,
) -> Result<Option<Value>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let probe: Probe = match raw {
        Value::String(text) => serde_json::from_str(&text),
        value => serde_json::from_value(value),
    }
    .map_err(|e| format!("Invalid first-token report: {e}"))?;
    probe
        .checked(
            call,
            body.ok_or("First-token report requires a captured request body.")?,
            excerpts,
        )
        .map(Some)
}

/// A span may declare a report whose payload is truncated or unreadable.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Captured {
    pub evidence: crate::Evidence,
    pub report: Option<Probe>,
}

/// Recheck the current captured body and named edits before interpreting any scores.
pub fn recorded(call: &Call, excerpts: bool) -> Option<Result<Value, String>> {
    let capture = call.prefill_probe.as_ref()?;
    Some((|| {
        let probe = capture
            .report
            .as_ref()
            .ok_or("The captured first-token report is unreadable.")?;
        let body = call
            .recorded_request
            .as_ref()
            .ok_or("The original request is unavailable for first-token verification.")?
            .parse()?;
        probe.checked(call, &body, excerpts)
    })())
}
