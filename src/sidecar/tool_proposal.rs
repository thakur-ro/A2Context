//! Join a captured provider proposal to its tool definition in the same model request.
use super::findings::{digest, occurrence, Observation};
use a2context::tool_contract::{Assessment, Catalog};
use a2context::{Call, Run};
use serde_json::{json, Value};

pub(super) fn findings(run: &Run, call: &Call) -> Vec<Observation> {
    let mut proposals = call
        .decisions
        .iter()
        .filter(|d| d.kind == "tool_call")
        .peekable();
    if proposals.peek().is_none() {
        return Vec::new();
    }
    let Some(request) = call.recorded_request.as_ref() else {
        return Vec::new();
    };
    let Ok(body) = serde_json::from_str::<Value>(&request.body) else {
        return Vec::new();
    };
    let catalog = Catalog::from_request(&body);
    proposals
        .filter_map(|decision| {
            let assessment = catalog.check(&decision.label, &decision.outcome);
            let Assessment::MissingRequired { ref schema_pointer, .. } = assessment else {
                return None;
            };
            let (schema, pointer) = catalog.schema(&decision.label)?;
            let location = schema_pointer.strip_prefix(pointer)?;
            let key = json!([run.source, run.organization_id, call.model, decision.label,
                digest(schema.to_string()), location]);
            let detail = json!({
                "tool": decision.label, "check": assessment, "decision": decision.id,
                "proposal_evidence": decision.evidence, "request_evidence": request.evidence, "sources": [],
            });
            Some(occurrence("missing_tool_argument", key, call, detail))
        })
        .collect()
}

#[derive(serde::Deserialize)]
pub(super) struct Submission {
    request: Value,
    tool: String,
    arguments: Value,
}

pub(super) fn check(input: Submission) -> Value {
    json!(Catalog::from_request(&input.request).check(&input.tool, &input.arguments))
}
