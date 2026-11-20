//! Serve preflight checks, context comparisons, and conflict breakdowns.

use nexusweave::{conflict_query, context_changes, Call, Decision, Run};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tiny_http::{Method, Request};

pub fn preflight(request: &mut Request) -> Option<(u16, Vec<u8>)> {
    if request.method() != &Method::Post || request.url() != "/api/preflight" {
        return None;
    }
    let result = crate::replay_http::read_json(request, 1024 * 1024)
        .and_then(|input| nexusweave::preflight::check(input).map_err(|error| (400, error)))
        .and_then(|report| {
            serde_json::to_vec(&report)
                .map_err(|_| (500, "Could not prepare the preflight report.".into()))
        });
    Some(match result {
        Ok(body) => (200, body),
        Err((status, message)) => (
            status,
            json!({ "message": message }).to_string().into_bytes(),
        ),
    })
}

pub fn pending(request: &mut Request) -> Option<(u16, Vec<u8>)> {
    if request.method() != &Method::Post || request.url() != "/api/pending" {
        return None;
    }
    Some(
        match crate::replay_http::read_json(request, 1024 * 1024)
            .and_then(|input| nexusweave::pending::inspect(input).map_err(|error| (400, error)))
        {
            Ok(report) => (200, report.to_string().into_bytes()),
            Err((status, message)) => (
                status,
                json!({ "message": message }).to_string().into_bytes(),
            ),
        },
    )
}

pub fn conflicts<'a>(
    request: &mut Request,
    runs: impl Iterator<Item = &'a Run>,
) -> Option<(u16, Vec<u8>)> {
    if request.method() != &Method::Post
        || request.url().split('?').next() != Some("/api/conflict-query")
    {
        return None;
    }
    let result = crate::replay_http::read_json::<conflict_query::ViewSpec>(request, 16 * 1024)
        .and_then(|spec| {
            let page = conflict_query::query(&spec, runs).map_err(|error| (400, error))?;
            serde_json::to_vec(&page)
                .map_err(|_| (500, "Could not prepare the conflict breakdown.".into()))
        });
    Some(match result {
        Ok(body) => (200, body),
        Err((status, message)) => (
            status,
            json!({ "message": message }).to_string().into_bytes(),
        ),
    })
}

pub fn route<'a>(request: &Request, runs: impl Iterator<Item = &'a Run>) -> Option<(u16, Vec<u8>)> {
    if request.method() != &Method::Get {
        return None;
    }
    let query = request.url().strip_prefix("/api/context-comparison?")?;
    let result = parse_query(query).and_then(|params| {
        let left_id = params.get("a_run")?;
        let right_id = params.get("b_run")?;
        let mut matching = runs.filter(|run| &run.id == left_id || &run.id == right_id);
        let first = matching.next()?;
        let (left_run, right_run) = if left_id == right_id {
            (first, first)
        } else if &first.id == left_id {
            (first, matching.find(|run| &run.id == right_id)?)
        } else {
            (matching.find(|run| &run.id == left_id)?, first)
        };
        let left = locate(&params, "a", left_run)?;
        let right = locate(&params, "b", right_run)?;
        let report = Comparison {
            outcome_status: context_changes::compare_outcomes(left.2, right.2),
            outcome_text: [readable(&left.2.outcome), readable(&right.2.outcome)],
            rule_text: [rules(left.1), rules(right.1)],
            context: context_changes::align((left.0, left.1), (right.0, right.1)),
        };
        Some(serde_json::to_vec(&report))
    });
    Some(match result {
        Some(Ok(body)) => (200, body),
        Some(Err(_)) => (
            500,
            br#"{"message":"Could not prepare the comparison."}"#.to_vec(),
        ),
        None => (
            404,
            br#"{"message":"A selected decision is unavailable."}"#.to_vec(),
        ),
    })
}

#[derive(Serialize)]
struct Comparison<'a> {
    outcome_status: context_changes::OutcomeStatus,
    outcome_text: [Option<String>; 2],
    rule_text: [Vec<RuleText<'a>>; 2],
    context: Vec<context_changes::Alignment<'a>>,
}

#[derive(Serialize)]
struct RuleText<'a> {
    context_id: &'a str,
    text: String,
}

fn rules(call: &Call) -> Vec<RuleText<'_>> {
    call.context
        .iter()
        .filter_map(|item| {
            let text = serde_json::to_string_pretty(item.rule.as_ref()?).ok()?;
            Some(RuleText {
                context_id: &item.id,
                text,
            })
        })
        .collect()
}

fn parse_query(query: &str) -> Option<BTreeMap<&str, String>> {
    let mut params = BTreeMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        let value = value.replace('+', " ");
        let value = percent_decode_str(&value).decode_utf8().ok()?.into_owned();
        if params.insert(key, value).is_some() {
            return None;
        }
    }
    Some(params)
}

fn locate<'a>(
    params: &BTreeMap<&str, String>,
    side: &str,
    run: &'a Run,
) -> Option<(&'a Run, &'a Call, &'a Decision)> {
    let call_id = params.get(format!("{side}_call").as_str())?;
    let decision_id = params.get(format!("{side}_decision").as_str())?;
    let call = run.calls.iter().find(|call| &call.id == call_id)?;
    let decision = call
        .decisions
        .iter()
        .find(|decision| &decision.id == decision_id)?;
    Some((run, call, decision))
}

fn readable(value: &Value) -> Option<String> {
    if value.is_null() {
        None
    } else if let Some(text) = value.as_str() {
        Some(text.to_owned())
    } else {
        serde_json::to_string_pretty(value).ok()
    }
}
