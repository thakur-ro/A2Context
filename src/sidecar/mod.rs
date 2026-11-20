mod capture;
mod findings;
mod history;
mod order;
mod outcomes;
mod position_plan;
mod prefill;
mod recommendations;
mod replacement;
mod screened;
mod store;
mod target_patterns;
mod targets;
mod variation;

use crate::{
    browser_asset, context_http, evidence_http, header, otlp_http, replay_http,
    serialize_inspection, Result,
};
use nexusweave::{activation, otlp::CapturePolicy, replay, Run};
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

struct Options {
    address: SocketAddr,
    db: Option<PathBuf>,
    retention: i64,
    demo: bool,
    content: bool,
    metadata: BTreeSet<String>,
}

impl Options {
    fn parse(mut args: &[&str]) -> Result<Self> {
        let mut options = Self {
            address: "127.0.0.1:8040".parse()?,
            db: None,
            retention: 7,
            demo: false,
            content: false,
            metadata: BTreeSet::new(),
        };
        while let Some((flag, rest)) = args.split_first() {
            args = match (*flag, rest) {
                ("--demo", rest) => {
                    options.demo = true;
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
            || options.demo == options.db.is_some()
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
    let options = Options::parse(args)?;
    let temp = options.demo.then(tempfile::tempdir).transpose()?;
    let db_path = options
        .db
        .clone()
        .or_else(|| temp.as_ref().map(|dir| dir.path().join("demo.sqlite")))
        .expect("sidecar storage");
    let mut writer = Store::open(&db_path)?;
    writer.bind_policy(options.content || options.demo, &options.metadata)?;
    writer.purge(now() - options.retention * 86_400)?;
    writer.upgrade_observations()?;
    if options.demo {
        seed(&mut writer)?;
    }
    let store = Store::open(&db_path)?;
    let server = Server::http(options.address)?;
    let address = server.server_addr().to_ip().expect("TCP listener");
    let hosts = [address.to_string(), format!("localhost:{}", address.port())];
    let metrics = Arc::new(Metrics::default());
    let (send, receive) = mpsc::sync_channel(QUEUE);
    let worker_metrics = Arc::clone(&metrics);
    let content = options.content;
    let retention = options.retention;
    let metadata = options.metadata.clone();
    let worker = std::thread::Builder::new()
        .name("nexusweave-findings".into())
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
    println!("NexusWeave sidecar is listening at http://{address}");
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
                    | "/api/position-results"
                    | "/api/replay-reports"
                    | "/api/activation-reports"
                    | "/api/sidecar/preview"
            )
        {
            if options.demo && request.url() != "/api/sidecar/preview" {
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
    loop {
        if last_purge.elapsed() >= Duration::from_secs(60) {
            if let Err(error) = store.purge(now() - retention * 86_400) {
                eprintln!("Sidecar retention: {error}");
            }
            last_purge = Instant::now();
        }
        let (enqueued, mut request) = match receive.recv_timeout(Duration::from_secs(5)) {
            Ok(task) => task,
            Err(RecvTimeoutError::Timeout) => continue,
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
        } else if request.url() == "/api/sidecar/preview" {
            let reply = replay_http::read_json(&mut request, LIMIT)
                .and_then(|input| recommendations::preview(&store, input));
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
            let reply = if request.url() == "/api/position-results" {
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
    let url = request.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let fields = query_fields(query);
    let result: Result<Option<Vec<u8>>> = (|| {
        if request.method() == &Method::Get {
            let value = match path {
                "/api/sidecar/health" => json!({
                    "demo": options.demo, "content_capture": options.content || options.demo,
                    "metadata_keys": options.metadata, "queue_depth": metrics.depth.load(Relaxed),
                    "queue_limit": QUEUE, "queue_drops": metrics.drops.load(Relaxed),
                    "processing_lag_ms": metrics.lag_ms.load(Relaxed),
                    "accepted_batches": metrics.accepted.load(Relaxed), "rejected_batches": metrics.rejected.load(Relaxed),
                    "coverage": store.coverage(&fields)?,
                }),
                "/api/sidecar/findings"
                | "/api/sidecar/traces"
                | "/api/sidecar/threads"
                | "/api/sidecar/impact"
                | "/api/sidecar/conversations-without-findings"
                | "/api/sidecar/target-patterns"
                | "/api/sidecar/target-examples"
                | "/api/sidecar/ranking-results"
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
                "/api/sidecar/token-path" if options.content || options.demo => {
                    store.token_path(&fields)?
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
                    let sources = findings::derive(&run, &reports, &activations)?;
                    let mut prefill_issues = BTreeMap::new();
                    let mut target_checks = BTreeMap::new();
                    for call in &mut run.calls {
                        if call.prefill_probe.is_some()
                            && !sources
                                .iter()
                                .any(|obs| obs.kind == "prefill" && obs.call_id == call.id)
                        {
                            if let Some(Err(issue)) =
                                nexusweave::replay::prefill::recorded(call, false)
                            {
                                prefill_issues.insert(call.id.clone(), issue);
                            }
                        }
                        if call.shadow_probes.is_some() {
                            let probes = replay::shadow::recorded(id, call);
                            let checks = targets::checks(call, &probes);
                            if !checks.is_empty() {
                                target_checks.insert(call.id.clone(), checks);
                            }
                        }
                        call.prefill_probe = None;
                        call.recorded_request = None;
                    }
                    json!({"trace":run, "target_checks":target_checks, "prefill_issues":prefill_issues, "findings":sources, "activation_reports": activations.len(),
                        "replay_reports": reports.len()})
                }
                "/api/sidecar/preview-example" => {
                    let fixture = match fields.get("scenario").map(String::as_str) {
                        Some(
                            scenario @ ("window-earlier" | "window-followup" | "window-unopened"
                            | "window-fresh"),
                        ) => journal_example(scenario.trim_start_matches("window-"))?,
                        Some("window-before") => {
                            let mut example = journal_example("earlier")?;
                            example
                                .as_object_mut()
                                .expect("saved envelope")
                                .remove("shadow");
                            example
                        }
                        Some("screen") => {
                            let sample: serde_json::Value = serde_json::from_str(include_str!(
                                "../../examples/context-screening/evidence.json"
                            ))?;
                            json!({
                                "request":serde_json::from_str::<serde_json::Value>(include_str!("../../examples/context-screening/request.json"))?,
                                "screening":sample[0]["calls"][0]["context_screening"]["report"]
                            })
                        }
                        Some("warranty") => {
                            serde_json::from_str::<serde_json::Value>(include_str!(
                                "../../examples/redundant-routing/warranty-probe-results.json"
                            ))?["example"]
                                .take()
                        }
                        Some("history") => json!({
                            "request":serde_json::from_str::<serde_json::Value>(include_str!("../../examples/redundant-routing/pending-maple.json"))?
                        }),
                        Some(scenario @ ("prior-only" | "scored-history" | "checked-history")) => {
                            let mut fixture: serde_json::Value = serde_json::from_str(include_str!(
                                "../../examples/redundant-routing/candidate-checks/ash-routing-note.json"
                            ))?;
                            if scenario != "scored-history" {
                                fixture
                                    .as_object_mut()
                                    .expect("demo envelope")
                                    .remove("candidate");
                            }
                            if scenario == "checked-history" {
                                let checks: serde_json::Value =
                                    serde_json::from_str(include_str!(
                                        "../../examples/redundant-routing/ash-history-check.json"
                                    ))?;
                                fixture["request"]["attributes"][replay::prefill::ATTRIBUTE] =
                                    checks["prefill"].clone();
                                fixture["shadow"] = checks["shadow"].clone();
                            }
                            fixture
                        }
                        Some("routing-test") => {
                            let records: Vec<serde_json::Value> = serde_json::from_str(include_str!(
                                "../../examples/runtime-comparison/pending-routing/shadow-cohort.json"
                            ))?;
                            let probes: serde_json::Value = serde_json::from_str(include_str!(
                                "../../examples/runtime-comparison/pending-routing/prefill-cohort.json"
                            ))?;
                            let id = "routing-south-stock-only";
                            let record = records
                                .iter()
                                .find(|r| r["id"] == id)
                                .ok_or("Missing routing example")?;
                            let meta = &record["extra"]["metadata"];
                            json!({"request":{"version":1,"attributes":{
                                "gen_ai.request.model":meta["ls_model_name"],
                                "nexusweave.request.body":meta["nexusweave.request.body"],
                                "nexusweave.context.semantics":meta["nexusweave.context.semantics"],
                                (replay::prefill::ATTRIBUTE):probes[id]
                            }}})
                        }
                        Some("prefill") => json!({
                            "request": serde_json::from_str::<serde_json::Value>(include_str!("../../examples/runtime-comparison/pending-routing/prefill-preview.json"))?,
                        }),
                        Some("routing") => json!({
                            "request": serde_json::from_str::<serde_json::Value>(include_str!("../../examples/runtime-comparison/pending-routing/request.json"))?,
                            "candidate": serde_json::from_str::<serde_json::Value>(include_str!("../../examples/runtime-comparison/pending-routing/report.json"))?,
                            "shadow": serde_json::from_str::<serde_json::Value>(include_str!("../../examples/runtime-comparison/pending-routing/shadow-report.json"))?,
                        }),
                        _ => json!({
                            "request": serde_json::from_str::<serde_json::Value>(include_str!("../../examples/preflight-returns.json"))?,
                        }),
                    };
                    let question = fixture["request"]["attributes"]["nexusweave.request.body"]
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
        let asset: Option<(&str, &[u8])> = match path {
            "/sidecar-token-path.js" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../../ui/sidecar-token-path.js"),
            )),
            "/sidecar.js" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../../ui/sidecar.js"),
            )),
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

fn journal_example(name: &str) -> Result<serde_json::Value> {
    if name == "fresh" {
        return Ok(serde_json::from_str(include_str!(
            "../../examples/context-screening/replay/fresh-target-tokens.json"
        ))?);
    }
    let mut envelope: serde_json::Value = serde_json::from_str(match name {
        "earlier" => include_str!("../../examples/context-screening/replay/earlier-target.json"),
        "followup" => include_str!("../../examples/context-screening/replay/followup-target.json"),
        "unopened" => include_str!("../../examples/context-screening/replay/unopened-target.json"),
        _ => return Err("Unknown journal experiment".into()),
    })?;
    let mut scores: serde_json::Value = serde_json::from_str(include_str!(
        "../../examples/context-screening/replay/target-token-scores.json"
    ))?;
    envelope["candidate"] = scores[name].take();
    Ok(envelope)
}

fn seed(store: &mut Store) -> Result<()> {
    let mut runs: Vec<Run> =
        serde_json::from_str(include_str!("../../fixtures/conversation.json"))?;
    let mut sessions: Vec<Run> = serde_json::from_str(include_str!("../../fixtures/support.json"))?;
    for (day, session) in sessions.iter_mut().enumerate() {
        for (hour, call) in session.calls.iter_mut().enumerate() {
            if call.started_at.is_none() {
                call.started_at = Some(format!("2026-10-{:02}T{:02}:00:00Z", 16 + day, 9 + hour));
            }
        }
    }
    runs.extend(sessions);
    runs.extend(serde_json::from_str::<Vec<Run>>(include_str!(
        "../../examples/context-screening/replay/evidence.json"
    ))?);
    runs.extend(serde_json::from_str::<Vec<Run>>(include_str!(
        "../../examples/redundant-routing/measured/evidence.json"
    ))?);
    #[derive(serde::Deserialize)]
    struct OrderExperiment {
        runs: Vec<Run>,
        reports: Vec<replay::Replay>,
    }
    let experiment: OrderExperiment = serde_json::from_str(include_str!(
        "../../examples/redundant-routing/measured/local-order.json"
    ))?;
    runs.extend(experiment.runs);
    let mut reports = experiment.reports;
    reports.extend([
        serde_json::from_str(include_str!(
            "../../examples/redundant-routing/measured/unavailable-report.json"
        ))?,
        serde_json::from_str(include_str!(
            "../../examples/redundant-routing/measured/neutral-report.json"
        ))?,
        serde_json::from_str(include_str!(
            "../../examples/context-screening/replay/report.json"
        ))?,
        serde_json::from_str(include_str!(
            "../../examples/context-screening/replay/report-policy.json"
        ))?,
    ]);
    // Wrap the saved pending experiments as synthetic LangSmith captures. The
    // proposals belong to the shadow runner. No production answer was recorded.
    for (question, name) in [
        ("first", "earlier"),
        ("follow-up", "followup"),
        ("unopened", "unopened"),
        ("fresh", "fresh"),
    ] {
        let envelope = journal_example(name)?;
        let attrs = &envelope["request"]["attributes"];
        let body = attrs["nexusweave.request.body"]
            .as_str()
            .ok_or("missing demo request")?;
        let capture = json!({"id":format!("journal-window-{question}"),"run_type":"llm",
            "name":"Journal return question","start_time":"2026-11-20T08:00:00Z",
            "inputs":serde_json::from_str::<serde_json::Value>(body)?,"extra":{"metadata":{
                "thread_id":format!("journal-window-{question}"),"organization_id":"journal-window-study",
                "user_id":"support-17","nexusweave.span.id":format!("journal-window-{question}"),"ls_model_name":attrs["gen_ai.request.model"],
                "nexusweave.request.body":body,"nexusweave.context.semantics":attrs["nexusweave.context.semantics"],
                "nexusweave.candidate_probes":[envelope["candidate"]],"nexusweave.shadow_probes":[envelope["shadow"]]}}});
        runs.extend(nexusweave::langsmith::import(&capture)?);
    }
    let pending = include_str!("../../examples/runtime-comparison/pending-routing/trace.json");
    runs.extend(nexusweave::langsmith::import(&serde_json::from_str(
        pending,
    )?)?);
    let pending_shadow =
        include_str!("../../examples/runtime-comparison/pending-routing/shadow-trace.json");
    runs.extend(nexusweave::langsmith::import(&serde_json::from_str(
        pending_shadow,
    )?)?);
    let routing_study = include_str!(
        "../../examples/runtime-comparison/pending-routing/routing-holdout-results.json"
    );
    let mut routing_runs = nexusweave::langsmith::import(&serde_json::from_str(routing_study)?)?;
    let sampled =
        include_str!("../../examples/runtime-comparison/pending-routing/sampled-results.json");
    routing_runs.extend(nexusweave::langsmith::import(&serde_json::from_str(
        sampled,
    )?)?);
    let activation: activation::Report = serde_json::from_str(include_str!(
        "../../examples/runtime-comparison/generation-branches-transformers.json"
    ))?;
    let measured_runs: Vec<Run> = serde_json::from_str(include_str!(
        "../../examples/runtime-comparison/evidence.json"
    ))?;
    runs.extend(
        measured_runs
            .into_iter()
            .filter(|run| run.id == activation.location().run_id),
    );
    let tool_report = serde_json::from_str(include_str!(
        "../../examples/local-mlx-routing/output-token-report.json"
    ))?;
    runs.extend(serde_json::from_str::<Vec<Run>>(include_str!(
        "../../examples/local-mlx-routing/evidence.json"
    ))?);
    store.seed(
        &runs,
        &routing_runs,
        &reports,
        &[activation, tool_report],
        now(),
    )
}
