//! Inspect one assembled inference request and any reports supplied for it.

use crate::{context_screening as screen, preflight, replay, Evidence};
use serde::Deserialize;
use serde_json::{json, Value};

/// A pending request with optional reports from independent local adapters.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request: preflight::Request,
    pub screening: Option<screen::Report>,
    pub candidate: Option<replay::candidates::Report>,
    pub shadow: Option<replay::shadow::Report>,
}

/// Normalize pending context, check declared rules, and verify supplied reports.
/// With no screen supplied, compare exact captured text. No runner is invoked.
/// The result includes captured inputs and exact request locations.
///
/// ```
/// use nexusweave::{pending, preflight};
///
/// let request: preflight::Request = serde_json::from_str(include_str!(concat!(
///     env!("CARGO_MANIFEST_DIR"),
///     "/examples/preflight-returns.json"
/// )))?;
/// let view = pending::inspect(pending::Request {
///     request,
///     screening: None,
///     candidate: None,
///     shadow: None,
/// })?;
/// assert_eq!(view["preflight"]["findings"][0]["kind"], "conflict");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn inspect(input: Request) -> Result<Value, String> {
    let (mut call, request, scope) = preflight::pending_call(input.request)?;
    let preflight = preflight::report_for_pending(&call, scope.clone());
    if let Some(report) = input.screening {
        call.context_screening = Some(screen::CapturedReport {
            evidence: anchor("pending.screening"),
            report: Some(report),
        });
    } else if call.context_screening.is_none() {
        call.context_screening = Some(screen::CapturedReport {
            evidence: anchor("pending.exact_text"),
            report: Some(screen::screen_call(
                &call,
                &screen::Selection::default(),
                |_| Ok(None),
            )?),
        });
    }
    if let Some(report) = input.candidate {
        call.candidate_probes = Some(replay::candidates::CapturedReports {
            evidence: anchor("pending.candidate"),
            reports: Some(vec![report]),
        });
    }
    if let Some(report) = input.shadow {
        call.shadow_probes = Some(replay::shadow::CapturedReports {
            evidence: anchor("pending.shadow"),
            reports: Some(vec![report]),
        });
    }
    let fields = request
        .as_ref()
        .map(|body| replay::request_field_matches(&call, body))
        .transpose()?;
    Ok(json!({
        "version": 1,
        "model": call.model,
        "context": call.context,
        "request_fields": fields,
        "preflight": preflight,
        "scope": scope,
        "screening": screen::recorded("pending", &call),
        "candidate_probes": replay::candidates::recorded("pending", &call),
        "shadow_probes": replay::shadow::recorded("pending", &call),
    }))
}

fn anchor(field: &str) -> Evidence {
    Evidence {
        span_id: None,
        field: field.into(),
        message_index: None,
        part_index: None,
    }
}
