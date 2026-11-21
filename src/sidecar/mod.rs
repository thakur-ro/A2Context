mod activity_search;
mod capability;
mod capture;
mod citation;
mod conflict;
mod conversation;
mod dispatch;
mod findings;
mod history;
mod opening;
mod order;
mod outcomes;
mod position_plan;
mod prefill;
mod prefix_patterns;
mod recommendations;
mod reference_links;
mod replacement;
mod screened;
mod shared_identifiers;
mod store;
mod target_patterns;
mod targets;
mod tool_handoffs;
mod tool_proposal;
mod tool_reference_score;
mod variation;

use crate::{
    browser_asset, context_http, evidence_http, header, otlp_http, replay_http,
    serialize_inspection, Result,
};
use a2context::{activation, otlp::CapturePolicy, replay, Run};
use percent_encoding::percent_decode_str;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use store::Store;
use time::OffsetDateTime;
use tiny_http::{Method, Request, Response, Server};

const LIMIT: usize = 2 * 1024 * 1024;
const QUEUE: usize = 8;

#[derive(Clone, Copy)]
enum Demo {
    Sidecar,
    Exchange,
}

struct Options {
    address: SocketAddr,
    db: Option<PathBuf>,
    retention: i64,
    demo: Option<Demo>,
    content: bool,
    metadata: BTreeSet<String>,
}

impl Options {
    fn parse(mut args: &[&str]) -> Result<Self> {
        let mut options = Self {
            address: "127.0.0.1:8040".parse()?,
            db: None,
            retention: 7,
            demo: None,
            content: false,
            metadata: BTreeSet::new(),
        };
        while let Some((flag, rest)) = args.split_first() {
            args = match (*flag, rest) {
                ("--demo", rest) => {
                    options.demo = Some(Demo::Sidecar);
                    rest
                }
                ("--capture-content", rest) => {
                    options.content = true;
                    rest
                }
                ("--listen", [value, rest @ ..]) => {
                    options.address = value.parse()?;
                    rest
                }
                ("--db", [value, rest @ ..]) => {
                    options.db = Some(PathBuf::from(value));
                    rest
                }
                ("--retain-days", [value, rest @ ..]) => {
                    options.retention = value.parse()?;
                    rest
                }
                ("--metadata", [value, rest @ ..]) if metadata_key(value) => {
                    options.metadata.insert((*value).into());
                    rest
                }
                _ => return Err(format!("unknown or incomplete sidecar option {flag}").into()),
            };
        }
        if !options.address.ip().is_loopback()
            || options.demo.is_some() == options.db.is_some()
            || !(1..=365).contains(&options.retention)
            || options.metadata.len() > 16
        {
            return Err("sidecar requires loopback, exactly one of --db or --demo, retention of 1–365 days and at most 16 metadata keys".into());
        }
        Ok(options)
    }
}

fn metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 100
        && ![
            "gen_ai.",
            "llm.",
            "input.",
            "output.",
            "tool.",
            "retrieval.",
            "a2context.",
            "nexusweave.",
        ]
        .iter()
        .any(|prefix| key.starts_with(prefix))
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[derive(Default)]
struct Metrics {
    depth: AtomicUsize,
    drops: AtomicUsize,
    lag_ms: AtomicUsize,
    accepted: AtomicUsize,
    rejected: AtomicUsize,
}

pub fn serve(args: &[&str]) -> Result<()> {
    start(Options::parse(args)?)
}

pub fn exchange_demo() -> Result<()> {
    let mut options = Options::parse(&["--demo", "--listen", "127.0.0.1:0"])?;
    options.demo = Some(Demo::Exchange);
    start(options)
}

fn start(mut options: Options) -> Result<()> {
    let demo = options.demo.is_some();
    let temp = demo.then(tempfile::tempdir).transpose()?;
    let db_path = options
        .db
        .clone()
        .or_else(|| temp.as_ref().map(|dir| dir.path().join("demo.sqlite")))
        .expect("sidecar storage");
    let mut writer = Store::open(&db_path)?;
    writer.bind_policy(options.content || demo, &options.metadata)?;
    writer.purge(now() - options.retention * 86_400)?;
    writer.upgrade_observations()?;
    if demo {
        let batch = serde_json::from_str(include_str!(
            "../../examples/exchange-approval/exchange.otlp.json"
        ))?;
        let policy = CapturePolicy {
            content: true,
            metadata: &options.metadata,
        };
        writer
            .ingest(&batch, &policy, now())
            .map_err(|(_, message)| message)?;
    }
    let store = Store::open(&db_path)?;
    let server = Server::http(options.address)?;
    let address = server.server_addr().to_ip().expect("TCP listener");
    options.address = address;
    let hosts = [address.to_string(), format!("localhost:{}", address.port())];
    let metrics = Arc::new(Metrics::default());
    let (send, receive) = mpsc::sync_channel(QUEUE);
    let worker_metrics = Arc::clone(&metrics);
    let content = options.content;
    let retention = options.retention;
    let metadata = options.metadata.clone();
    let worker = std::thread::Builder::new()
        .name("a2context-findings".into())
        .spawn(move || {
            work(
                writer,
                receive,
                worker_metrics,
                content,
                metadata,
                retention,
            )
        })?;
    if matches!(options.demo, Some(Demo::Exchange)) {
        println!("Open http://{address}/");
    } else {
        println!("A2Context sidecar is listening at http://{address}");
    }
    for mut request in server.incoming_requests() {
        if !local(&request, &hosts) {
            respond(
                request,
                403,
                "text/plain",
                b"Local origin required".to_vec(),
            );
            continue;
        }
        if request.method() == &Method::Post
            && matches!(
                request.url(),
                "/v1/traces"
                    | "/api/prefix-results"
                    | "/api/position-results"
                    | "/api/replay-reports"
                    | "/api/activation-reports"
                    | "/api/sidecar/preview"
                    | "/api/sidecar/check-proposal"
            )
        {
            if demo
                && !matches!(
                    request.url(),
                    "/api/sidecar/preview" | "/api/sidecar/check-proposal"
                )
            {
                respond(
                    request,
                    405,
                    "text/plain",
                    b"Demo uses saved evidence".to_vec(),
                );
                continue;
            }
            metrics.depth.fetch_add(1, Relaxed);
            if let Err(error) = send.try_send((Instant::now(), request)) {
                metrics.depth.fetch_sub(1, Relaxed);
                metrics.drops.fetch_add(1, Relaxed);
                let (_, request) = match error {
                    TrySendError::Full(value) | TrySendError::Disconnected(value) => value,
                };
                respond(
                    request,
                    503,
                    "text/plain",
                    b"Sidecar queue unavailable".to_vec(),
                );
            }
            continue;
        }
        let reply = route(&mut request, &store, &metrics, &options);
        respond(request, reply.0, reply.1, reply.2);
    }
    drop(send);
    let _ = worker.join();
    Ok(())
}

fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

fn work(
    mut store: Store,
    receive: Receiver<(Instant, Request)>,
    metrics: Arc<Metrics>,
    content: bool,
    metadata: BTreeSet<String>,
    retention: i64,
) {
    let policy = CapturePolicy {
        content,
        metadata: &metadata,
    };
    let mut last_purge = Instant::now();
    let mut index_wait = Duration::from_millis(25);
    loop {
        if last_purge.elapsed() >= Duration::from_secs(60) {
            if let Err(error) = store.purge(now() - retention * 86_400) {
                eprintln!("Sidecar retention: {error}");
            }
            last_purge = Instant::now();
        }
        let (enqueued, mut request) = match receive.recv_timeout(index_wait) {
            Ok(task) => task,
            Err(RecvTimeoutError::Timeout) => {
                if index_wait < Duration::from_secs(5) {
                    match store.backfill_activity_search() {
                        Ok(true) => {}
                        Ok(false) => index_wait = Duration::from_secs(5),
                        Err(error) => {
                            eprintln!("Background activity indexing: {error}");
                            index_wait = Duration::from_secs(1);
                        }
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        metrics.depth.fetch_sub(1, Relaxed);
        if request.url() == "/v1/traces" {
            let accepted = otlp_http::receive(request, LIMIT, |document| {
                store.ingest(&document, &policy, now())
            });
            let metric = if accepted {
                &metrics.accepted
            } else {
                &metrics.rejected
            };
            metric.fetch_add(1, Relaxed);
            metrics
                .lag_ms
                .store(enqueued.elapsed().as_millis() as usize, Relaxed);
        } else if matches!(
            request.url(),
            "/api/sidecar/preview" | "/api/sidecar/check-proposal"
        ) {
            let reply = if request.url() == "/api/sidecar/check-proposal" {
                replay_http::read_json(&mut request, LIMIT).map(tool_proposal::check)
            } else {
                replay_http::read_json(&mut request, LIMIT)
                    .and_then(|input| recommendations::preview(&store, input))
            };
            let (status, body) = match reply {
                Ok(body) => (200, body),
                Err((status, message)) => (status, json!({"message": message})),
            };
            respond(
                request,
                status,
                "application/json",
                body.to_string().into_bytes(),
            );
        } else {
            let reply = if request.url() == "/api/prefix-results" {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Submission {
                    scope: BTreeMap<String, String>,
                    shadow: replay::shadow::Report,
                }
                if content {
                    replay_http::read_json::<Submission>(&mut request, LIMIT)
                        .and_then(|s| store.attach_prefix(s.scope, s.shadow))
                } else {
                    Err((403, "Prefix trials require content capture.".into()))
                }
            } else if request.url() == "/api/position-results" {
                replay_http::read_json::<position_plan::Submission>(&mut request, LIMIT)
                    .and_then(|submitted| store.attach_position(submitted))
            } else if request.url() == "/api/replay-reports" {
                replay_http::read_json::<replay::Replay>(&mut request, LIMIT)
                    .and_then(|report| store.attach(&report, now()))
            } else {
                replay_http::read_json::<activation::Report>(&mut request, LIMIT)
                    .and_then(|report| store.attach(&report, now()))
            };
            let (status, body) = match reply {
                Ok(created) => (
                    if created { 201 } else { 200 },
                    json!({ "created": created }),
                ),
                Err((status, message)) => (status, json!({ "message": message })),
            };
            respond(
                request,
                status,
                "application/json",
                body.to_string().into_bytes(),
            );
        }
    }
}

fn local(request: &Request, hosts: &[String]) -> bool {
    let headers = request.headers();
    headers.iter().any(|header| {
        header.field.equiv("Host") && hosts.iter().any(|host| host == header.value.as_str())
    }) && !headers.iter().any(|header| {
        (header.field.equiv("Sec-Fetch-Site") && header.value.as_str() == "cross-site")
            || (header.field.equiv("Origin")
                && !hosts
                    .iter()
                    .any(|host| header.value.as_str().strip_prefix("http://") == Some(host)))
    })
}

type Reply = (u16, &'static str, Vec<u8>);

fn route(request: &mut Request, store: &Store, metrics: &Metrics, options: &Options) -> Reply {
    let demo = options.demo.is_some();
    let url = request.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let fields = query_fields(query);
    let result: Result<Option<Vec<u8>>> = (|| {
        if request.method() == &Method::Get {
            let value = match path {
                "/api/sidecar/chat-config" => json!({"models":[]}),
                "/api/sidecar/workflows" => json!({"workflows":store.workflows(&fields)?}),
                "/api/sidecar/tool-reference-score" => store.tool_reference_score(&fields)?,
                "/api/sidecar/activity-search" => store.search_activity(&fields)?,
                "/api/sidecar/recent-tool-names" => store.recent_tool_names(&fields)?,
                "/api/sidecar/health" => json!({
                    "demo": demo, "content_capture": options.content || demo,
                    "otlp_http_endpoint": format!("http://{}/v1/traces", options.address),
                    "metadata_keys": options.metadata, "queue_depth": metrics.depth.load(Relaxed),
                    "queue_limit": QUEUE, "queue_drops": metrics.drops.load(Relaxed),
                    "processing_lag_ms": metrics.lag_ms.load(Relaxed),
                    "accepted_batches": metrics.accepted.load(Relaxed), "rejected_batches": metrics.rejected.load(Relaxed),
                    "coverage": store.coverage(&fields)?,
                }),
                "/api/sidecar/findings"
                | "/api/sidecar/traces"
                | "/api/sidecar/source-history"
                | "/api/sidecar/threads"
                | "/api/sidecar/thread-calls"
                | "/api/sidecar/impact"
                | "/api/sidecar/summary"
                | "/api/sidecar/conflict-examples"
                | "/api/sidecar/conflict-coverage"
                | "/api/sidecar/conflict-conversations"
                | "/api/sidecar/dispatch-summary"
                | "/api/sidecar/dispatch-conversations"
                | "/api/sidecar/dispatch-designs"
                | "/api/sidecar/conversations-without-findings"
                | "/api/sidecar/prefix-reference-pairs"
                | "/api/sidecar/prefix-contrasts"
                | "/api/sidecar/prefix-groups"
                | "/api/sidecar/prefix-sources"
                | "/api/sidecar/prefix-references"
                | "/api/sidecar/prefix-cases"
                | "/api/sidecar/target-patterns"
                | "/api/sidecar/target-examples"
                | "/api/sidecar/ranking-results"
                | "/api/sidecar/opening-coverage"
                | "/api/sidecar/opening-sources"
                | "/api/sidecar/opening-conversations"
                | "/api/sidecar/ranking-review"
                | "/api/sidecar/ranking-reference"
                | "/api/sidecar/ranking-examples"
                | "/api/sidecar/screened-pairs"
                | "/api/sidecar/screened-pair-examples"
                | "/api/sidecar/outcome-factors"
                | "/api/sidecar/conversation-outcomes"
                | "/api/sidecar/proposal-conversations"
                | "/api/sidecar/factors"
                | "/api/sidecar/order-tests"
                | "/api/sidecar/replay-variations"
                | "/api/sidecar/occurrences" => {
                    store.list(path.rsplit('/').next().expect("view path"), &fields)?
                }
                "/api/sidecar/token-path" if options.content || demo => {
                    store.token_path(&fields)?
                }
                "/api/sidecar/prefix-input" if options.content => {
                    let Some(input) = store.prefix_input(&fields)? else {
                        return Ok(None);
                    };
                    input
                }
                "/api/sidecar/prefix-check" => {
                    let Some(mut check) = store.prefix_check(&fields)? else {
                        return Ok(None);
                    };
                    check["capture_enabled"] = options.content.into();
                    check
                }
                "/api/sidecar/target-check" => {
                    let Some(check) = store.target_check(&fields)? else {
                        return Ok(None);
                    };
                    check
                }
                "/api/sidecar/position-results" => store.position_results(&fields)?,
                "/api/sidecar/position-plan" => {
                    return Ok(Some(serde_json::to_vec(&store.position_plan(&fields)?)?))
                }
                "/api/sidecar/trace" => {
                    let id = required(&fields, "id")?;
                    let Some(mut run) = store.trace(id)? else {
                        return Ok(None);
                    };
                    let reports = store.replays(id)?;
                    let activations = store.activations(id)?;
                    let mut sources = findings::derive(&run, &reports, &activations)?;
                    sources.extend(store.shared_identifier(&run)?);
                    let mut prefill_issues = BTreeMap::new();
                    let mut target_checks = BTreeMap::new();
                    let mut prefix_reviews = BTreeMap::new();
                    let tool_reference_issue = fields
                        .get("call")
                        .and_then(|id| {
                            run.calls
                                .iter()
                                .find(|c| c.id == *id || c.span_id.as_ref() == Some(id))
                        })
                        .and_then(tool_reference_score::witness);
                    for call in &mut run.calls {
                        if call.prefill_probe.is_some()
                            && !sources
                                .iter()
                                .any(|obs| obs.kind == "prefill" && obs.call_id == call.id)
                        {
                            if let Some(Err(issue)) =
                                a2context::replay::prefill::recorded(call, false)
                            {
                                prefill_issues.insert(call.id.clone(), issue);
                            }
                        }
                        if call.shadow_probes.is_some() || call.candidate_probes.is_some() {
                            let probes = replay::shadow::recorded(id, call);
                            let reviews = replay::candidates::prefix_reviews(
                                &replay::candidates::recorded(id, call),
                                &probes,
                            );
                            if !reviews.is_empty() {
                                prefix_reviews.insert(call.id.clone(), reviews);
                            }
                            let checks = targets::checks(call, &probes);
                            if !checks.is_empty() {
                                target_checks.insert(call.id.clone(), checks);
                            }
                        }
                        call.prefill_probe = None;
                        call.recorded_request = None;
                    }
                    let selected = match fields.get("call") {
                        Some(id) => run
                            .calls
                            .iter()
                            .find(|call| call.id == *id || call.span_id.as_ref() == Some(id)),
                        None => run.calls.first(),
                    };
                    let received_results = selected
                        .map(|call| tool_handoffs::received_by(&run, call))
                        .unwrap_or_default();
                    json!({"trace":run, "tool_reference_issue":tool_reference_issue, "received_tool_results":received_results, "prefix_reviews":prefix_reviews, "target_checks":target_checks, "prefill_issues":prefill_issues, "findings":sources, "activation_reports": activations.len(),
                        "replay_reports": reports.len()})
                }
                "/api/sidecar/preview-example" => {
                    let raw = match fields.get("scenario").map(String::as_str) {
                        Some("exchange-approval") => {
                            include_str!("../../examples/exchange-approval/pending.json")
                        }
                        Some("policy") | None => {
                            include_str!("../../examples/preflight-returns.json")
                        }
                        _ => return Ok(None),
                    };
                    let fixture =
                        json!({"request":serde_json::from_str::<serde_json::Value>(raw)?});
                    let question = fixture["request"]["attributes"]["a2context.request.body"]
                        .as_str()
                        .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                        .and_then(|body| {
                            let last = body["messages"].as_array()?.last()?;
                            (last["role"] == "user")
                                .then(|| last["content"].as_str().map(str::to_owned))
                                .flatten()
                        });
                    let mut result =
                        recommendations::preview(store, serde_json::from_value(fixture)?)
                            .map_err(|(_, message)| message)?;
                    result["example_question"] = json!(question);
                    result
                }
                "/api/inspection" => {
                    let details = detail_scope(store, &fields)?;
                    return Ok(Some(serialize_inspection(
                        &details.runs,
                        &details.replays,
                        &details.activations,
                        false,
                        false,
                        true,
                    )?));
                }
                _ => return Ok(None),
            };
            return Ok(Some(serde_json::to_vec(&value)?));
        }
        Ok(None)
    })();
    match result {
        Ok(Some(body)) => return (200, "application/json", body),
        Err(error) => {
            return (
                400,
                "application/json",
                json!({"message":error.to_string()})
                    .to_string()
                    .into_bytes(),
            )
        }
        Ok(None) => (),
    }
    if request.method() == &Method::Get && path == "/" {
        return (
            200,
            "text/html; charset=utf-8",
            include_bytes!("../../ui/sidecar.html").to_vec(),
        );
    }
    if request.method() == &Method::Get {
        // Serve one script on the dashboard's cold path. Keep individual asset URLs for older pages.
        const SCRIPTS: &[(&str, &[u8])] = &[
            (
                "/sidecar-a2ui.js",
                include_bytes!("../../ui/sidecar-a2ui.js"),
            ),
            (
                "/sidecar-prefix.js",
                include_bytes!("../../ui/sidecar-prefix.js"),
            ),
            (
                "/sidecar-token-path.js",
                include_bytes!("../../ui/sidecar-token-path.js"),
            ),
            (
                "/context-network.js",
                include_bytes!("../../ui/context-network.js"),
            ),
            (
                "/interview-conversations.js",
                include_bytes!("../../ui/interview-conversations.js"),
            ),
            (
                "/interview-plan.js",
                include_bytes!("../../ui/interview-plan.js"),
            ),
            (
                "/sidecar-canvas.js",
                include_bytes!("../../ui/sidecar-canvas.js"),
            ),
            (
                "/source-coverage.js",
                include_bytes!("../../ui/source-coverage.js"),
            ),
            (
                "/tool-catalog.js",
                include_bytes!("../../ui/tool-catalog.js"),
            ),
            ("/sidecar.js", include_bytes!("../../ui/sidecar.js")),
        ];
        const JS: &str = "text/javascript; charset=utf-8";
        if path == "/workspace.js" {
            let mut body = Vec::with_capacity(SCRIPTS.iter().map(|(_, s)| s.len() + 3).sum());
            for (_, script) in SCRIPTS {
                body.extend_from_slice(script);
                body.extend_from_slice(b"\n;\n");
            }
            return (200, JS, body);
        }
        if let Some((_, script)) = SCRIPTS.iter().find(|(url, _)| *url == path) {
            return (200, JS, script.to_vec());
        }
        let asset: Option<(&str, &[u8])> = match path {
            "/sidecar.css" => Some((
                "text/css; charset=utf-8",
                include_bytes!("../../ui/sidecar.css"),
            )),
            "/details" => Some((
                "text/html; charset=utf-8",
                include_bytes!("../../ui/index.html"),
            )),
            _ => None,
        };
        if let Some((mime, body)) = asset {
            return (200, mime, body.to_vec());
        }
        if let Some((status, mime, body)) = browser_asset(path) {
            return (status, mime, body.to_vec());
        }
    }
    if let Some((status, body)) = context_http::pending(request) {
        return (status, "application/json", body);
    }
    let mut detail_fields = fields;
    if path == "/api/context-comparison" {
        if let (Some(left), Some(right)) = (detail_fields.get("a_run"), detail_fields.get("b_run"))
        {
            let (left, right) = (left.clone(), right.clone());
            detail_fields.insert("run".into(), left);
            detail_fields.insert("other".into(), right);
        }
    }
    if path.starts_with("/api/")
        && (detail_fields.contains_key("run")
            || detail_fields.contains_key("thread")
            || detail_fields.contains_key("legacy"))
    {
        if let Ok(details) = detail_scope(store, &detail_fields) {
            let reply = replay_http::route(request, details.runs.iter())
                .or_else(|| context_http::route(request, details.runs.iter()))
                .or_else(|| {
                    evidence_http::route(
                        request,
                        details.runs.iter(),
                        &details.replays,
                        &details.activations,
                    )
                })
                .or_else(|| context_http::conflicts(request, details.runs.iter()));
            if let Some((status, body)) = reply {
                return (status, "application/json", body);
            }
        }
    }
    (404, "text/plain", b"Not found".to_vec())
}

struct Details {
    runs: Vec<Run>,
    replays: Vec<replay::Replay>,
    activations: Vec<activation::Report>,
}

fn detail_scope(store: &Store, fields: &BTreeMap<String, String>) -> Result<Details> {
    let runs: Vec<_> = if let Some(id) = fields.get("run") {
        let right = fields.get("other").filter(|right| *right != id);
        std::iter::once(id)
            .chain(right)
            .map(|id| {
                store
                    .trace(id)?
                    .ok_or_else(|| "trace is unavailable in this workspace".into())
            })
            .collect::<Result<_>>()?
    } else if fields.contains_key("thread") || fields.contains_key("legacy") {
        store.legacy_runs(fields.get("thread").map(String::as_str))?
    } else {
        return Err("choose a trace or thread".into());
    };
    let mut details = Details {
        runs,
        replays: Vec::new(),
        activations: Vec::new(),
    };
    for run in &details.runs {
        details.replays.extend(store.replays(&run.id)?);
        details.activations.extend(store.activations(&run.id)?);
    }
    Ok(details)
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    Ok(fields.get(key).ok_or_else(|| format!("missing {key}"))?)
}

fn query_fields(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let decoded = |text: &str| {
                percent_decode_str(&text.replace('+', " "))
                    .decode_utf8()
                    .map(|value| value.into_owned())
                    .ok()
            };
            Some((decoded(key)?, decoded(value)?))
        })
        .collect()
}

fn respond(request: Request, status: u16, content_type: &str, bytes: Vec<u8>) {
    let _ = request.respond(
        Response::from_data(bytes)
            .with_status_code(status)
            .with_header(header("Content-Type", content_type))
            .with_header(header("Cache-Control", "no-store"))
            .with_header(header("X-Content-Type-Options", "nosniff"))
            .with_header(header(
                "Content-Security-Policy",
                "default-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'",
            )),
    );
}
