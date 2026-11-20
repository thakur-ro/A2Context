//! Inspect one assembled inference request and any reports supplied for it.

use crate::{
    context_screening as screen, preflight, replay, turn_evidence, Call, DecisionScope, Evidence,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A pending request with optional reports from independent local adapters.
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request: preflight::Request,
    pub screening: Option<screen::Report>,
    pub candidate: Option<replay::candidates::Report>,
    pub shadow: Option<replay::shadow::Report>,
}

/// Paginated pending evidence, with source text available through an explicit opt-in.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequest {
    pub input: Request,
    #[serde(default)]
    pub include_excerpts: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
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
    let Inspected {
        call,
        request,
        scope,
        preflight,
        prefill,
    } = prepare(input, true)?;
    let fields = request
        .as_ref()
        .map(|body| replay::request_field_matches(&call, body))
        .transpose()?;
    let candidates = replay::candidates::recorded("pending", &call);
    let shadows = replay::shadow::recorded("pending", &call);
    Ok(json!({
        "version": 1, "prefill":prefill,
        "prefix_reviews":replay::candidates::prefix_reviews(&candidates, &shadows),
        "model": call.model,
        "context": call.context,
        "request_fields": fields,
        "preflight": preflight,
        "scope": scope,
        "screening": screen::recorded("pending", &call),
        "candidate_probes": candidates,
        "shadow_probes": shadows,
    }))
}

/// Inspect only the request and reports supplied by the caller. By default, return
/// source locations and measurements with captured text and values omitted.
pub fn evidence(query: EvidenceRequest, allow_excerpts: bool) -> Result<Value, String> {
    if query.include_excerpts && !allow_excerpts {
        return Err("Captured excerpts are disabled for this MCP server.".into());
    }
    if !(1..=50).contains(&query.limit) {
        return Err("Choose a page size from 1 to 50.".into());
    }
    let Inspected {
        call,
        request,
        scope,
        preflight,
        prefill,
    } = prepare(query.input, query.include_excerpts)?;
    let excerpts = query.include_excerpts;
    let fields: BTreeMap<_, _> = match request.as_ref() {
        Some(body) => replay::request_field_matches(&call, body)?
            .into_iter()
            .map(|item| (item.context_id, item.fields))
            .collect(),
        None => BTreeMap::new(),
    };
    let screening = screen::recorded("pending", &call);
    let flagged_pairs = match screening.as_ref().map(|item| &item.verification) {
        Some(screen::Verification::Matched { report }) => report
            .pairs
            .iter()
            .filter(|pair| pair.suggestion.is_some())
            .count(),
        _ => 0,
    };
    let candidates = replay::candidates::recorded("pending", &call);
    let shadows = replay::shadow::recorded("pending", &call);
    let mut rows: Vec<_> = turn_evidence::preflight_rows(&preflight, excerpts)
        .chain(turn_evidence::screen_rows(screening.as_ref(), &call, excerpts).take(flagged_pairs))
        .chain(
            candidates
                .iter()
                .flat_map(|probe| turn_evidence::candidate_rows(probe, excerpts)),
        )
        .chain(
            shadows
                .iter()
                .flat_map(|probe| turn_evidence::shadow_rows(probe, excerpts)),
        )
        .chain(call.context.iter().map(|item| {
            json!({
                "kind": "captured_input", "input": turn_evidence::context_ref(item, excerpts),
                "request_fields": fields.get(item.id.as_str()),
            })
        }))
        .skip(query.offset)
        .take(query.limit + 1)
        .collect();
    let next = (rows.len() > query.limit).then(|| query.offset.saturating_add(query.limit));
    rows.truncate(query.limit);
    Ok(json!({
        "kind": "pending_evidence", "version": 1, "prefill":prefill, "model": call.model,
        "captured_inputs": call.context.len(), "original_request_captured": request.is_some(),
        "captured_excerpts_enabled": allow_excerpts,
        "preflight": {"scope": scope, "finding_count": preflight.findings.len(),
            "scope_coverage": preflight.scope_coverage, "scoped_rule_items": preflight.scoped_rule_items, "receipt": preflight.receipt},
        "context_screening": screening.as_ref().map(turn_evidence::screen_summary),
        "candidate_probes": candidates.iter().map(|probe| turn_evidence::candidate_summary(probe, excerpts)).collect::<Vec<_>>(),
        "shadow_probes": shadows.iter().map(|probe| turn_evidence::shadow_summary(probe, excerpts)).collect::<Vec<_>>(),
        "rows": rows, "next_offset": next,
    }))
}

/// Normalize a pending request and its supplied probes for recommendation logic.
/// The call retains captured input and must be treated as sensitive.
pub fn recommendation_input(input: Request) -> Result<(Call, Option<DecisionScope>), String> {
    let (call, scope, _) = recommendation_measurements(input)?;
    Ok((call, scope))
}

/// Return verified first-token edit measurements alongside the pending call.
pub fn recommendation_measurements(
    input: Request,
) -> Result<(Call, Option<DecisionScope>, Option<Value>), String> {
    let Inspected {
        call,
        scope,
        prefill,
        ..
    } = prepare(input, true)?;
    Ok((call, scope, prefill))
}

struct Inspected {
    prefill: Option<Value>,
    call: Call,
    request: Option<Value>,
    scope: Option<DecisionScope>,
    preflight: preflight::Report,
}

fn prepare(mut input: Request, excerpts: bool) -> Result<Inspected, String> {
    let raw = input.request.attributes.remove(replay::prefill::ATTRIBUTE);
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
    let prefill = replay::prefill::read(raw, &call, request.as_ref(), excerpts)?;
    Ok(Inspected {
        prefill,
        call,
        request,
        scope,
        preflight,
    })
}

fn anchor(field: &str) -> Evidence {
    Evidence {
        span_id: None,
        field: field.into(),
        message_index: None,
        part_index: None,
    }
}
