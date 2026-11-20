//! Serve paginated evidence queries over the inspector's current data.

use nexusweave::{
    activation, context_screening, replay, replay_effects, source_profiles, turn_evidence, Run,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tiny_http::{Method, Request};

type Reply = (u16, Vec<u8>);

pub fn route<'a>(
    request: &mut Request,
    runs: impl Iterator<Item = &'a Run>,
    replays: &'a [replay::Replay],
    reports: &'a [activation::Report],
) -> Option<Reply> {
    if request.method() != &Method::Post {
        return None;
    }
    // The loopback inspector already displays the underlying capture. These queries
    // return excerpts only when the request asks for them.
    let reply = match request.url().split('?').next()? {
        "/api/turn-evidence" => query(request, |input| {
            turn_evidence::query_with_activations(input, runs, replays, reports, true)
        }),
        "/api/activation-effects" => query(request, |input| {
            activation::patterns::query::query(input, runs, reports, true)
        }),
        "/api/output-token-effects" => query(request, |input| {
            activation::output_scores::query(input, reports, true)
        }),
        "/api/generation-branches" => query(request, |input| {
            activation::generation_branches::query(input, reports, true)
        }),
        "/api/source-visibility" => query(request, |input| {
            activation::visibility::query::query(input, reports, true)
        }),
        "/api/visibility-patterns" => query(request, |input| {
            activation::visibility::patterns::query(input, runs, reports)
        }),
        "/api/candidate-patterns" => query(request, |input| {
            replay::candidates::patterns::query(input, runs)
        }),
        "/api/shadow-patterns" => query(request, |input| {
            replay::shadow::patterns::query(input, runs, true)
        }),
        "/api/screening-patterns" => query(request, |input| {
            context_screening::patterns::query_with_replays(input, runs, replays, true)
        }),
        "/api/replay-effects" => query(request, |input| {
            replay_effects::query::query(input, runs, replays, true)
        }),
        "/api/context-sources" => query(request, |input| {
            source_profiles::query::query(input, runs, replays, reports)
        }),
        _ => return None,
    };
    Some(reply)
}

fn query<T: DeserializeOwned>(
    request: &mut Request,
    read: impl FnOnce(&T) -> Result<Value, String>,
) -> Reply {
    let (status, body) = match crate::replay_http::read_json(request, 16 * 1024)
        .and_then(|input| read(&input).map_err(|error| (400, error)))
    {
        Ok(body) => (200, body),
        Err((status, message)) => (status, json!({ "message": message })),
    };
    (status, body.to_string().into_bytes())
}
