//! Normalize the OpenTelemetry span rows published by Exgentic.

use crate::genai::{decoded, empty_call, optional_str, record_inference};
use crate::{Run, SCHEMA_VERSION};
use serde_json::Value;

pub fn import_row(row: &Value) -> Result<Run, String> {
    let session = required_str(row, "session_id")?;
    let benchmark = required_str(row, "benchmark")?;
    if benchmark != "tau2_retail" {
        return Err(format!("unsupported Exgentic benchmark {benchmark}"));
    }
    let raw_spans = decoded(row.get("spans"), "spans")?;
    let spans = raw_spans.as_array().ok_or("spans must be an array")?;
    let mut calls = Vec::new();
    for span in spans {
        let span = decoded(Some(span), "span")?;
        let attrs = decoded(span.get("attributes"), "attributes")?;
        if attrs.get("gen_ai.input.messages").is_none()
            && attrs.get("gen_ai.output.messages").is_none()
        {
            continue;
        }
        let call = empty_call(
            required_str(&span, "span_id")?,
            optional_str(&span, "trace_id"),
            optional_str(&span, "start_time"),
        );
        calls.push(record_inference(call, &attrs)?);
    }
    calls.sort_by(|a, b| (&a.started_at, &a.id).cmp(&(&b.started_at, &b.id)));
    let run = Run {
        schema_version: SCHEMA_VERSION,
        id: format!("exgentic:{session}"),
        source: format!("exgentic:{benchmark}"),
        organization_id: None,
        user_id: None,
        conversation_id: Some(session.to_owned()),
        calls,
        spans: Vec::new(),
    };
    crate::validate(&run)?;
    Ok(run)
}

fn required_str<'a>(object: &'a Value, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{field} must be a nonempty string"))
}
