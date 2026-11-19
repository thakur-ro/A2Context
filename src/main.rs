use nexusweave::{
    activation, add_annotations, context_screening, exgentic, inspect, langsmith, otlp, preflight,
    replay, replay_effects, Annotation, Evidence, Run,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};
use tiny_http::{Header, Method, Response, Server};

const USAGE: &str = "Usage:
  nexusweave import <exgentic.jsonl> <evidence.json>
  nexusweave import-langsmith <runs.json|runs.jsonl> <evidence.json>
  nexusweave import-otlp <traces.json|traces.jsonl> <evidence.json>
  nexusweave annotate <evidence.json> <annotations.json> <output.json>
  nexusweave replay <evidence.json> <case.json> <report.json> -- <runner> [args...]
  nexusweave rescore-replay <evidence.json> <report.json> <match-rule.json> <new-id> <output.json>
  nexusweave validate <evidence.json> [...]
  nexusweave preflight <request.json|->
  nexusweave screen-context <request.json> <selection.json|-> <report.json> [-- <scorer> [args...]]
  nexusweave screen-recorded-context <evidence.json> <run-id> <call-id> <selection.json|-> <output.json> [-- <scorer> [args...]]
  nexusweave probe-candidates <request.json> <probe.json> <report.json> -- <scorer> [args...]
  nexusweave shadow <request.json> <case.json> <report.json> -- <runner> [args...]
  nexusweave query-conflicts <view.json> <evidence.json> [...]
  nexusweave query-candidate-patterns <view.json> <evidence.json> [...]
  nexusweave query-shadow-patterns <view.json> <evidence.json> [...]
  nexusweave query-screening-patterns <view.json> [--replay <report.json>]... <evidence.json> [...]
  nexusweave mcp-stdio [--scope scope.json] [--include-excerpts] [--replay report.json] [--activation-report report.json] <evidence.json> [...]
  nexusweave mcp-stdio [--scope scope.json] [--include-excerpts] --otlp-store directory [evidence.json ...]
  nexusweave inspect [--replay report.json] [--activation-report report.json] <evidence.json> [...]
  nexusweave serve [--listen 127.0.0.1:8040] [--replay report.json] [--activation-report report.json] [--otlp-store directory] [evidence.json ...]
    Supply evidence files, an OTLP store, or both.";

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["--help" | "-h"] => println!("{USAGE}"),
        ["import", input, output] => {
            let rows: Vec<Value> = read_jsonl(input)?;
            if rows.is_empty() {
                return Err("no Exgentic sessions found".into());
            }
            let runs = rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    exgentic::import_row(row)
                        .map_err(|error| format!("{input}:{}: {error}", index + 1))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            inspect(&runs)?;
            write_new(output, &serde_json::to_vec_pretty(&runs)?)?;
            println!("Imported {} sessions to {output}.", runs.len());
        }
        [command @ ("import-langsmith" | "import-otlp"), input, output] => {
            let document = read_json_or_jsonl(input)?;
            let (import, format) = if *command == "import-otlp" {
                (
                    otlp::import as fn(&Value) -> std::result::Result<Vec<Run>, String>,
                    "OTLP",
                )
            } else {
                (langsmith::import as _, "LangSmith")
            };
            let runs = import(&document).map_err(|error| format!("{input}: {error}"))?;
            inspect(&runs)?;
            write_new(output, &serde_json::to_vec_pretty(&runs)?)?;
            println!("Imported {} {format} traces to {output}.", runs.len());
        }
        ["annotate", input, annotations, output] => {
            let mut runs = read_runs(&[input])?;
            let annotations: Vec<Annotation> =
                serde_json::from_reader(fs::File::open(annotations)?)?;
            add_annotations(&mut runs, annotations)?;
            inspect(&runs)?;
            write_new(output, &serde_json::to_vec_pretty(&runs)?)?;
        }
        ["replay", input, specification, output, "--", program, runner_args @ ..] => {
            if Path::new(output).exists() {
                return Err(format!("replay output already exists: {output}").into());
            }
            let runs = read_runs(&[input])?;
            inspect(&runs)?;
            let case: replay::Case = serde_json::from_reader(fs::File::open(specification)?)?;
            let report = replay::execute(&case, &runs, |request| {
                sample(program, runner_args, request)
            })?;
            write_new(output, &serde_json::to_vec_pretty(&report)?)?;
            println!(
                "Recorded {} baseline trials and {} variant conditions to {output}.",
                report.baseline.len(),
                report.variants.len() + usize::from(report.pair.is_some())
            );
        }
        ["rescore-replay", input, report, rule, new_id, output] => {
            let runs = read_runs(&[input])?;
            inspect(&runs)?;
            let report: replay::Replay = serde_json::from_reader(fs::File::open(report)?)?;
            let rule: replay::MatchRule = serde_json::from_reader(fs::File::open(rule)?)?;
            let rescored = replay::rescore(report, &runs, new_id, rule)?;
            write_new(output, &serde_json::to_vec_pretty(&rescored)?)?;
            println!("Rescored recorded trials to {output}.");
        }
        ["validate", paths @ ..] if !paths.is_empty() => {
            let runs = read_runs(paths)?;
            let report = inspect(&runs)?;
            println!("Validated {} runs.", report.runs.len());
        }
        ["preflight", path] => {
            let request = if *path == "-" {
                serde_json::from_reader(std::io::stdin().lock())?
            } else {
                serde_json::from_reader(fs::File::open(path)?)?
            };
            let report = preflight::check(request)?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
            println!();
        }
        ["screen-context", request, selection, output, args @ ..] => {
            let screen = ContextScreen::from_args(selection, output, args)?;
            let request = serde_json::from_reader(fs::File::open(request)?)?;
            let report = context_screening::screen_pending(request, &screen.selection, |batch| {
                screen.score(batch)
            })?;
            write_new(output, &serde_json::to_vec_pretty(&report)?)?;
            println!(
                "Screened {} of {} context pairs to {output}.",
                report.pairs.len(),
                report.total_pairs
            );
        }
        ["screen-recorded-context", input, run_id, call_id, selection, output, args @ ..] => {
            let screen = ContextScreen::from_args(selection, output, args)?;
            let mut runs = read_runs(&[input])?;
            validate_loaded_runs(&runs)?;
            let call = runs
                .iter_mut()
                .find(|run| run.id == *run_id)
                .and_then(|run| run.calls.iter_mut().find(|call| call.id == *call_id))
                .ok_or("The selected context screening call is unavailable.")?;
            let report = context_screening::screen_call(call, &screen.selection, |batch| {
                screen.score(batch)
            })?;
            let (screened, total) = (report.pairs.len(), report.total_pairs);
            call.context_screening = Some(context_screening::CapturedReport {
                evidence: Evidence {
                    span_id: call.span_id.clone(),
                    field: "local.context_screening".into(),
                    message_index: None,
                    part_index: None,
                },
                report: Some(report),
            });
            write_new(output, &serde_json::to_vec_pretty(&runs)?)?;
            println!(
                "Screened {screened} of {total} context pairs and saved evidence to {output}."
            );
        }
        ["probe-candidates", request, specification, output, "--", program, scorer_args @ ..] => {
            if Path::new(output).exists() {
                return Err(format!("candidate probe output already exists: {output}").into());
            }
            let request = serde_json::from_reader(fs::File::open(request)?)?;
            let case = serde_json::from_reader(fs::File::open(specification)?)?;
            let report = replay::candidates::execute(request, &case, |input| {
                run_json(program, scorer_args, input, Stdio::inherit())
                    .map_err(|error| error.to_string())
            })?;
            write_new(output, &serde_json::to_vec_pretty(&report)?)?;
            println!(
                "Measured {} source edits to {output}.",
                report.effects.len()
            );
        }
        ["shadow", request, specification, output, "--", program, runner_args @ ..] => {
            if Path::new(output).exists() {
                return Err(format!("shadow report already exists: {output}").into());
            }
            let request = serde_json::from_reader(fs::File::open(request)?)?;
            let case = serde_json::from_reader(fs::File::open(specification)?)?;
            let report = replay::shadow::execute(request, &case, |request| {
                match run_json(program, runner_args, request, Stdio::inherit()) {
                    Ok(choice) => Some(choice),
                    Err(error) => {
                        eprintln!("Shadow trial failed. {error}");
                        None
                    }
                }
            })?;
            write_new(output, &serde_json::to_vec_pretty(&report)?)?;
            println!(
                "Recorded shadow proposals for {} source edits to {output}.",
                report.effects.len()
            );
        }
        ["query-conflicts", spec, paths @ ..] if !paths.is_empty() => {
            let runs = read_runs(paths)?;
            validate_loaded_runs(&runs)?;
            let spec = serde_json::from_reader(fs::File::open(spec)?)?;
            let page = nexusweave::conflict_query::query(&spec, &runs)?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &page)?;
            println!();
        }
        [command @ ("query-candidate-patterns" | "query-shadow-patterns"), spec, paths @ ..]
            if !paths.is_empty() =>
        {
            let runs = read_runs(paths)?;
            validate_loaded_runs(&runs)?;
            let spec = serde_json::from_reader(fs::File::open(spec)?)?;
            let page = if *command == "query-candidate-patterns" {
                replay::candidates::patterns::query(&spec, &runs)?
            } else {
                replay::shadow::patterns::query(&spec, &runs, true)?
            };
            serde_json::to_writer_pretty(std::io::stdout().lock(), &page)?;
            println!();
        }
        ["query-screening-patterns", spec, args @ ..] => {
            let options = inspection_options(args, false)?;
            if !options.activation_reports.is_empty() {
                return Err(USAGE.into());
            }
            let runs = read_runs(options.evidence)?;
            validate_loaded_runs(&runs)?;
            let replays = read_replays(&options.reports, &runs)?;
            let spec = serde_json::from_reader(fs::File::open(spec)?)?;
            let page =
                context_screening::patterns::query_with_replays(&spec, &runs, &replays, true)?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &page)?;
            println!();
        }
        ["mcp-stdio", args @ ..] => mcp_stdio::serve(args)?,
        ["inspect", args @ ..] => {
            let options = inspection_options(args, false)?;
            use std::io::Write;
            std::io::stdout()
                .lock()
                .write_all(&inspection_json(&options, true)?)?;
            println!();
        }
        ["serve", args @ ..] => serve(inspection_options(args, true)?)?,
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

struct ContextScreen<'a> {
    selection: context_screening::Selection,
    scorer: Option<(&'a str, &'a [&'a str])>,
}

impl<'a> ContextScreen<'a> {
    fn from_args(selection: &str, output: &str, args: &'a [&'a str]) -> Result<Self> {
        if Path::new(output).exists() {
            return Err(format!("context screening output already exists: {output}").into());
        }
        let scorer = match args {
            [] => None,
            ["--", program, arguments @ ..] => Some((*program, arguments)),
            _ => return Err("Pass the scorer command and its arguments after --.".into()),
        };
        let selection = if selection == "-" {
            context_screening::Selection::default()
        } else {
            serde_json::from_reader(fs::File::open(selection)?)?
        };
        Ok(Self { selection, scorer })
    }

    fn score(
        &self,
        batch: &context_screening::ScoringRequest<'_>,
    ) -> std::result::Result<Option<context_screening::ScoringResult>, String> {
        self.scorer
            .map(|(program, args)| run_json(program, args, batch, Stdio::inherit()))
            .transpose()
            .map_err(|error| error.to_string())
    }
}

fn sample(program: &str, arguments: &[&str], request: &Value) -> Option<replay::Choice> {
    run_json(program, arguments, request, Stdio::null()).ok()
}

fn run_json<T: DeserializeOwned>(
    program: &str,
    arguments: &[&str],
    request: &impl Serialize,
    stderr: Stdio,
) -> Result<T> {
    use std::io::Write;
    let input = serde_json::to_vec(request)?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()?;
    let sent = child
        .stdin
        .take()
        .expect("piped child stdin")
        .write_all(&input);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(format!("{program} exited with {}", output.status).into());
    }
    sent?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn read_jsonl<T: DeserializeOwned>(path: &str) -> Result<Vec<T>> {
    let input = fs::read_to_string(path)?;
    parse_jsonl(path, &input)
}

fn read_json_or_jsonl(path: &str) -> Result<Value> {
    let input = fs::read_to_string(path)?;
    serde_json::from_str(&input).or_else(|_| parse_jsonl(path, &input).map(Value::Array))
}

fn parse_jsonl<T: DeserializeOwned>(path: &str, input: &str) -> Result<Vec<T>> {
    input
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{path}:{}: {error}", index + 1).into())
        })
        .collect()
}

fn read_runs(paths: &[&str]) -> Result<Vec<Run>> {
    let mut runs = Vec::new();
    for path in paths {
        let mut records: Vec<Run> =
            serde_json::from_reader(std::io::BufReader::new(fs::File::open(path)?))
                .map_err(|error| format!("{path}: {error}"))?;
        runs.append(&mut records);
    }
    if runs.is_empty() {
        return Err("no evidence runs found".into());
    }
    Ok(runs)
}

fn validate_loaded_runs(runs: &[Run]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for run in runs {
        nexusweave::validate(run)?;
        if !seen.insert(&run.id) {
            return Err(format!("duplicate run ID: {}", run.id).into());
        }
    }
    Ok(())
}

fn write_new(path: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let path = Path::new(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let written = file.write_all(bytes).and_then(|_| file.write_all(b"\n"));
    if let Err(error) = written {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

struct InspectionOptions<'a> {
    address: &'a str,
    evidence: &'a [&'a str],
    reports: Vec<&'a str>,
    activation_reports: Vec<&'a str>,
    otlp_store: Option<&'a str>,
}

fn inspection_options<'a>(
    mut options: &'a [&'a str],
    allow_listen: bool,
) -> Result<InspectionOptions<'a>> {
    let mut address = "127.0.0.1:8040";
    let mut reports = Vec::new();
    let mut activation_reports = Vec::new();
    let mut otlp_store = None;
    while let [flag, value, rest @ ..] = options {
        match *flag {
            "--listen" if allow_listen => address = value,
            "--otlp-store" if allow_listen => otlp_store = Some(*value),
            "--replay" => reports.push(*value),
            "--activation-report" => activation_reports.push(*value),
            _ => break,
        }
        options = rest;
    }
    if (options.is_empty() && otlp_store.is_none())
        || options.first().is_some_and(|path| path.starts_with('-'))
    {
        return Err(USAGE.into());
    }
    Ok(InspectionOptions {
        address,
        evidence: options,
        reports,
        activation_reports,
        otlp_store,
    })
}

#[derive(Serialize)]
struct InspectionSnapshot<'a> {
    #[serde(flatten)]
    inspection: nexusweave::Inspection<'a>,
    replays: &'a [replay::Replay],
    #[serde(skip_serializing_if = "<[activation::Report]>::is_empty")]
    activation_reports: &'a [activation::Report],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    activation_patterns: Vec<activation::patterns::Group<'a>>,
    replay_outcomes: Vec<replay::OutcomeDistribution<'a>>,
    replay_effects: Vec<replay_effects::EffectGroup<'a>>,
    context_replays: Vec<replay_effects::ContextEffects<'a>>,
    live_otlp: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recorded_requests: Vec<RequestLocation>,
}

#[derive(Serialize)]
struct RequestLocation {
    run_id: String,
    call_id: String,
}

fn read_replays(reports: &[&str], runs: &[Run]) -> Result<Vec<replay::Replay>> {
    let mut unique = BTreeMap::new();
    for path in reports {
        let report: replay::Replay = serde_json::from_reader(fs::File::open(path)?)
            .map_err(|error| format!("{path}: {error}"))?;
        replay::validate_report(&report, runs).map_err(|error| format!("{path}: {error}"))?;
        let key = (report.location.clone(), report.id.clone());
        if let Some(previous) = unique.get(&key) {
            if previous != &report {
                return Err(format!(
                    "{path}: conflicting replay {} for the same decision",
                    report.id
                )
                .into());
            }
        } else {
            unique.insert(key, report);
        }
    }
    Ok(unique.into_values().collect())
}

fn inspection_json(options: &InspectionOptions<'_>, pretty: bool) -> Result<Vec<u8>> {
    let runs = read_runs(options.evidence)?;
    let replays = read_replays(&options.reports, &runs)?;
    let activations = read_activation_reports(&options.activation_reports, &runs)?;
    serialize_inspection(&runs, &replays, &activations, pretty, false, false)
}

fn read_activation_reports(paths: &[&str], runs: &[Run]) -> Result<Vec<activation::Report>> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ReportFile {
        Batch(Vec<activation::Report>),
        Single(Box<activation::Report>),
    }

    let mut unique = BTreeMap::new();
    for path in paths {
        let file: ReportFile = serde_json::from_reader(fs::File::open(path)?)
            .map_err(|error| format!("{path}: {error}"))?;
        let reports = match file {
            ReportFile::Batch(reports) => reports,
            ReportFile::Single(report) => vec![*report],
        };
        for report in reports {
            report
                .validate(runs)
                .map_err(|error| format!("{path}: {}: {error}", report.id()))?;
            let key = (report.location().clone(), report.id().to_owned());
            match unique.entry(key) {
                std::collections::btree_map::Entry::Occupied(previous)
                    if previous.get() != &report =>
                {
                    return Err(
                        format!("{path}: conflicting activation report {}", report.id()).into(),
                    );
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(report);
                }
            }
        }
    }
    Ok(unique.into_values().collect())
}

fn serialize_inspection(
    runs: &[Run],
    replays: &[replay::Replay],
    activation_reports: &[activation::Report],
    pretty: bool,
    live_otlp: bool,
    hide_requests: bool,
) -> Result<Vec<u8>> {
    let mut inspection = inspect(runs)?;
    let replay_effects = replay_effects::group(&inspection.runs, replays)?;
    let activation_patterns = activation::patterns::group(&inspection.runs, activation_reports);
    let recorded_requests: Vec<_> = inspection
        .capture_profiles
        .iter()
        .filter(|profile| hide_requests && profile.request_evidence.is_some())
        .map(|profile| RequestLocation {
            run_id: profile.run_id.to_owned(),
            call_id: profile.call_id.to_owned(),
        })
        .collect();
    let redacted_runs: Vec<_> = if !hide_requests {
        Vec::new()
    } else {
        inspection
            .runs
            .iter()
            .map(|&run| {
                if !run.calls.iter().any(|call| {
                    call.recorded_request.is_some()
                        || call.candidate_probes.is_some()
                        || call.shadow_probes.is_some()
                        || call.context_screening.is_some()
                }) {
                    return Cow::Borrowed(run);
                }
                let mut run = run.clone();
                for call in &mut run.calls {
                    call.recorded_request = None;
                    call.candidate_probes = None;
                    call.shadow_probes = None;
                    call.context_screening = None;
                }
                Cow::Owned(run)
            })
            .collect()
    };
    if !redacted_runs.is_empty() {
        inspection.runs = redacted_runs.iter().map(Cow::as_ref).collect();
    }
    let report = InspectionSnapshot {
        inspection,
        replays,
        activation_reports,
        activation_patterns,
        replay_outcomes: replay::outcome_distributions(replays)?,
        context_replays: replay_effects::by_context(&replay_effects),
        replay_effects,
        live_otlp,
        recorded_requests,
    };
    if pretty {
        Ok(serde_json::to_vec_pretty(&report)?)
    } else {
        Ok(serde_json::to_vec(&report)?)
    }
}

fn inspection_runs<'a>(fixed: &'a [Run], journal: Option<&otlp_http::Journal>) -> Cow<'a, [Run]> {
    if let Some(journal) = journal {
        Cow::Owned(fixed.iter().chain(journal.runs()).cloned().collect())
    } else {
        Cow::Borrowed(fixed)
    }
}

fn serve(options: InspectionOptions<'_>) -> Result<()> {
    let address: SocketAddr = options.address.parse()?;
    if !address.ip().is_loopback() {
        return Err("the inspector requires a loopback address".into());
    }
    let fixed = if options.evidence.is_empty() {
        Vec::new()
    } else {
        read_runs(options.evidence)?
    };
    for run in &fixed {
        nexusweave::validate(run)?;
    }
    let fixed_ids: BTreeSet<_> = fixed.iter().map(|run| run.id.clone()).collect();
    let mut journal = options
        .otlp_store
        .map(|path| otlp_http::Journal::open(Path::new(path)))
        .transpose()?;
    if let Some(run) = journal
        .iter()
        .flat_map(|journal| journal.runs())
        .find(|run| fixed_ids.contains(&run.id))
    {
        return Err(format!(
            "live trace {} is already loaded from an evidence file",
            run.id
        )
        .into());
    }
    let initial = inspection_runs(&fixed, journal.as_ref());
    let replays = read_replays(&options.reports, &initial)?;
    let mut activations = read_activation_reports(&options.activation_reports, &initial)?;
    let activation_store = options
        .otlp_store
        .map(|path| activation_http::Store::open(Path::new(path), &initial, &mut activations))
        .transpose()?;
    let mut data = Some(serialize_inspection(
        &initial,
        &replays,
        &activations,
        false,
        journal.is_some(),
        true,
    )?);
    let server = Server::http(address)?;
    let bound = server.server_addr().to_ip().expect("bound a TCP socket");
    let allowed_hosts = [bound.to_string(), format!("localhost:{}", bound.port())];
    let allowed_origins = allowed_hosts
        .each_ref()
        .map(|host| format!("http://{host}"));
    println!("NexusWeave is listening at http://{bound}");
    if journal.is_some() {
        println!("Receiving OTLP traces at http://{bound}/v1/traces");
    }

    for mut request in server.incoming_requests() {
        let host = request
            .headers()
            .iter()
            .find(|header| header.field.equiv("Host"));
        let valid_host = host.is_some_and(|header| {
            allowed_hosts
                .iter()
                .any(|host| host == header.value.as_str())
        });
        let cross_origin = request.headers().iter().any(|header| {
            header.field.equiv("Sec-Fetch-Site") && header.value.as_str() == "cross-site"
                || header.field.equiv("Origin")
                    && !allowed_origins
                        .iter()
                        .any(|origin| origin == header.value.as_str())
        });
        let local = valid_host && !cross_origin;
        if local && request.method() == &Method::Post && request.url() == "/v1/traces" {
            if let Some(journal) = &mut journal {
                if journal.receive(request, &fixed_ids) {
                    data = None;
                }
                continue;
            }
        }
        if local
            && request.method() == &Method::Get
            && request.url() == "/api/inspection"
            && data.is_none()
        {
            let current = inspection_runs(&fixed, journal.as_ref());
            match serialize_inspection(
                &current,
                &replays,
                &activations,
                false,
                journal.is_some(),
                true,
            ) {
                Ok(snapshot) => data = Some(snapshot),
                Err(error) => eprintln!("Could not prepare current inspection: {error}"),
            }
        }
        let inspection_reply = if local {
            let runs = || {
                fixed
                    .iter()
                    .chain(journal.iter().flat_map(|journal| journal.runs()))
            };
            activation_store
                .as_ref()
                .and_then(|store| {
                    let previous = activations.len();
                    let reply = store.route(&mut request, runs(), &mut activations);
                    if activations.len() != previous {
                        data = None;
                    }
                    reply
                })
                .or_else(|| context_http::route(&request, runs()))
                .or_else(|| context_http::preflight(&mut request))
                .or_else(|| context_http::conflicts(&mut request, runs()))
                .or_else(|| evidence_http::route(&mut request, runs(), &replays, &activations))
                .or_else(|| replay_http::route(&mut request, runs()))
        } else {
            None
        };
        let (status, content_type, bytes) = match (local, request.method(), request.url()) {
            (false, _, _) => (403, "text/plain", b"Local origin required".as_slice()),
            (true, &Method::Get, "/") => (
                200,
                "text/html; charset=utf-8",
                include_bytes!("../ui/index.html").as_slice(),
            ),
            (true, &Method::Get, "/app.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/app.js").as_slice(),
            ),
            (true, &Method::Get, "/replay-planner.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/replay-planner.js").as_slice(),
            ),
            (true, &Method::Get, "/context-matrix.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/context-matrix.js").as_slice(),
            ),
            (true, &Method::Get, "/source-profiles.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/source-profiles.js").as_slice(),
            ),
            (true, &Method::Get, "/execution-tree.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/execution-tree.js").as_slice(),
            ),
            (true, &Method::Get, "/retrievals.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/retrievals.js").as_slice(),
            ),
            (true, &Method::Get, "/compare.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/compare.js").as_slice(),
            ),
            (true, &Method::Get, "/rule-findings.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/rule-findings.js").as_slice(),
            ),
            (true, &Method::Get, "/replay-outcomes.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/replay-outcomes.js").as_slice(),
            ),
            (true, &Method::Get, "/capture-profiles.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/capture-profiles.js").as_slice(),
            ),
            (true, &Method::Get, "/inspection-links.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/inspection-links.js").as_slice(),
            ),
            (true, &Method::Get, "/context-changes.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/context-changes.js").as_slice(),
            ),
            (true, &Method::Get, "/decision-changes.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/decision-changes.js").as_slice(),
            ),
            (true, &Method::Get, "/decision-investigation.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/decision-investigation.js").as_slice(),
            ),
            (true, &Method::Get, "/context-replay-coverage.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/context-replay-coverage.js").as_slice(),
            ),
            (true, &Method::Get, "/conversation.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/conversation.js").as_slice(),
            ),
            (true, &Method::Get, "/context-screening.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/context-screening.js").as_slice(),
            ),
            (true, &Method::Get, "/activation.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/activation.js").as_slice(),
            ),
            (true, &Method::Get, "/source-masks.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/source-masks.js").as_slice(),
            ),
            (true, &Method::Get, "/activation-output.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/activation-output.js").as_slice(),
            ),
            (true, &Method::Get, "/activation-branches.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/activation-branches.js").as_slice(),
            ),
            (true, &Method::Get, "/activation-patterns.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/activation-patterns.js").as_slice(),
            ),
            (true, &Method::Get, "/conflict-breakdown.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/conflict-breakdown.js").as_slice(),
            ),
            (true, &Method::Get, "/visibility-patterns.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/visibility-patterns.js").as_slice(),
            ),
            (true, &Method::Get, "/assessment-patterns.js") => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../ui/assessment-patterns.js").as_slice(),
            ),
            (true, &Method::Get, "/styles.css") => (
                200,
                "text/css; charset=utf-8",
                include_bytes!("../ui/styles.css").as_slice(),
            ),
            (true, &Method::Get, "/api/inspection") => match data.as_deref() {
                Some(snapshot) => (200, "application/json", snapshot),
                None => (
                    500,
                    "text/plain",
                    b"Could not prepare current inspection".as_slice(),
                ),
            },
            (true, _, _) if inspection_reply.is_some() => {
                let (status, body) = inspection_reply
                    .as_ref()
                    .expect("inspection route response");
                (*status, "application/json", body.as_slice())
            }
            (_, _, _) => (404, "text/plain", b"Not found".as_slice()),
        };
        let response = Response::new(status.into(), vec![
            header("Content-Type", content_type),
            header("Cache-Control", "no-store"),
            header("X-Content-Type-Options", "nosniff"),
            header("Content-Security-Policy", "default-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
        ], bytes, Some(bytes.len()), None);
        let _ = request.respond(response);
    }
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("static valid header")
}
mod activation_http;
mod context_http;
mod evidence_http;
mod mcp_stdio;
mod otlp_http;
mod replay_http;
