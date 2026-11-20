//! Inspect a request or validate a replay case without invoking a runner.

use nexusweave::{replay, Call, Run};
use percent_encoding::percent_decode_str;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use tiny_http::{Method, Request};

const MAX_BODY: usize = 64 * 1024 * 1024;
type Reply = Result<Vec<u8>, (u16, String)>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreRequest {
    case: replay::Case,
    batch: replay::batch::Batch,
}

pub fn route<'a>(
    request: &mut Request,
    runs: impl Iterator<Item = &'a Run>,
) -> Option<(u16, Vec<u8>)> {
    let path = request.url().split('?').next()?;
    let result = match (request.method(), path) {
        (&Method::Post, "/api/replay-check") => read_json::<replay::Case>(request, MAX_BODY)
            .and_then(|case| {
                replay::validate_case(&case, runs).map_err(|error| (400, error))?;
                encode(&json!({}))
            }),
        (&Method::Post, "/api/replay-plan") => read_json::<replay::Case>(request, MAX_BODY)
            .and_then(|case| {
                let plan = replay::batch::prepare(&case, runs).map_err(|error| (400, error))?;
                encode(&plan)
            }),
        (&Method::Post, "/api/replay-score") => read_json::<ScoreRequest>(request, MAX_BODY)
            .and_then(|input| {
                let report = replay::batch::score(&input.case, runs, input.batch)
                    .map_err(|error| (400, error))?;
                encode(&report)
            }),
        (&Method::Post, "/api/shadow-check") => {
            read_json::<replay::shadow::Plan>(request, 1024 * 1024).and_then(|plan| {
                replay::shadow::validate_case(plan.request, &plan.case)
                    .map_err(|error| (400, error))?;
                encode(&json!({}))
            })
        }
        (&Method::Get, "/api/replay-input" | "/api/replay-fields")
        | (&Method::Post, "/api/replay-fields") => match target(request.url(), runs) {
            Some((_, call)) if request.method() == &Method::Post => {
                read_json::<Value>(request, MAX_BODY).and_then(|body| fields(call, &body, None))
            }
            Some((run, call)) if call.recorded_request.is_some() => {
                let evidence = call_evidence(run, call);
                if path == "/api/replay-input" {
                    encode(&evidence)
                } else {
                    let body = &call
                        .recorded_request
                        .as_ref()
                        .expect("captured request")
                        .body;
                    serde_json::from_str(body)
                        .map_err(|_| {
                            (
                                400,
                                "The captured request could not be parsed as JSON.".into(),
                            )
                        })
                        .and_then(|body| fields(call, &body, Some(evidence)))
                }
            }
            _ => Err((
                404,
                "The selected call or captured request could not be found.".into(),
            )),
        },
        _ => return None,
    };
    Some(match result {
        Ok(bytes) => (200, bytes),
        Err((status, message)) => (
            status,
            json!({ "message": message }).to_string().into_bytes(),
        ),
    })
}

fn target<'a>(url: &str, mut runs: impl Iterator<Item = &'a Run>) -> Option<(&'a Run, &'a Call)> {
    let (_, query) = url.split_once("?run=")?;
    let (run_id, call_id) = query.split_once("&call=")?;
    let run_id = percent_decode_str(run_id).decode_utf8().ok()?;
    let call_id = percent_decode_str(call_id).decode_utf8().ok()?;
    let run = runs.find(|run| run.id == run_id)?;
    let call = run.calls.iter().find(|call| call.id == call_id)?;
    Some((run, call))
}

fn call_evidence(run: &Run, call: &Call) -> [Run; 1] {
    [Run {
        schema_version: run.schema_version,
        id: run.id.clone(),
        source: run.source.clone(),
        organization_id: run.organization_id.clone(),
        user_id: run.user_id.clone(),
        conversation_id: run.conversation_id.clone(),
        calls: vec![call.clone()],
        spans: Vec::new(),
    }]
}

fn fields(call: &Call, request: &Value, evidence: Option<[Run; 1]>) -> Reply {
    #[derive(Serialize)]
    struct Plan<'a> {
        fields: Vec<replay::ContextFieldMatch<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence_json: Option<String>,
    }
    let fields = replay::request_field_matches(call, request).map_err(|error| (400, error))?;
    // Keep the downloadable evidence byte-for-byte JSON, including wide integers.
    let evidence_json = evidence
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| (500, "Could not prepare the call evidence.".into()))?;
    encode(&Plan {
        fields,
        evidence_json,
    })
}

fn encode(value: &impl Serialize) -> Reply {
    serde_json::to_vec(value).map_err(|_| (500, "Could not prepare the replay data.".into()))
}

pub(crate) fn read_json<T: DeserializeOwned>(
    request: &mut Request,
    max_body: usize,
) -> Result<T, (u16, String)> {
    let json_content = request.headers().iter().any(|header| {
        header.field.equiv("Content-Type")
            && header
                .value
                .as_str()
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json_content {
        return Err((415, "Send the request as application/json.".into()));
    }
    let too_large = || (413, format!("The request exceeds {max_body} bytes."));
    if request.body_length().is_some_and(|len| len > max_body) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take((max_body + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| (400, "The request could not be read.".into()))?;
    if body.len() > max_body {
        return Err(too_large());
    }
    serde_json::from_slice(&body).map_err(|error| {
        (
            400,
            format!("The request could not be parsed as JSON. {error}"),
        )
    })
}
