use super::findings::{self, Observation};
use a2context::{activation, pending, replay, Availability, Call, Run};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct Action {
    title: &'static str,
    next: &'static str,
    basis: &'static str,
}

fn checked_source_fragments(
    call: &Call,
    coverage: &a2context::preflight::SourceCoverage,
) -> Vec<Value> {
    call.context
        .iter()
        .filter(|item| item.availability == Availability::Included)
        .filter_map(|item| {
            let source = item.source_id.as_ref()?;
            let range = item.request_fragment.as_ref()?;
            let expected = coverage.expected_text_sha256.get(source)?;
            let captured = findings::digest(item.text.as_ref()?);
            Some(json!({
                "id": item.id, "source": source, "version": item.source_version,
                "request_fragment": range, "captured_text_sha256": captured, "matches": captured == *expected,
            }))
        })
        .collect()
}

/// Recommend a follow-up only from the eligible trials on this occurrence.
pub(super) fn conflict_action(test: &Value) -> Action {
    let mut outcomes = test["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|source| source["comparisons"].as_array().into_iter().flatten())
        .filter_map(|comparison| comparison["changed"].as_bool());
    if outcomes.clone().any(|changed| changed) {
        Action {
            title: "Repeat the edit that changed the proposal",
            next: "Open a changed comparison to check its replacement and outcome. Repeat that edit on a fresh request with the same rule scope before updating the workflow.",
            basis: "Declared disagreement with a measured change on this call",
        }
    } else if outcomes.next().is_some() {
        Action {
            title: "Review which edits preserved the proposal",
            next: "Check the tested replacements and try the other declaration or a different wording. A matching proposal applies to that comparison.",
            basis: "Declared disagreement with matching replayed proposals",
        }
    } else {
        action("conflict", &Value::Null)
    }
}

pub fn action(kind: &str, detail: &Value) -> Action {
    let (title, next, basis) = match kind {
        "replay" if detail["joint_pattern"] == "joint_only" => (
            "Retest both sources before removing either",
            "Each single edit preserved the original proposal. Editing both changed it. Repeat all four conditions with a different request and compare the complete answers or tool arguments.",
            "Complete proposals from the same four-condition experiment",
        ),
        "replay" if detail["joint_only"] == true => (
            "Keep one source available while testing removal",
            "Repeat both single edits and the joint edit with a second replacement. A different wording can change the result.",
            "Only the joint edit changed the measured match rate",
        ),
        "replacement" => (
            "Repeat the comparison with both wordings",
            "Vary the wording and the order of the sources on fresh requests. Keep the scoring rule fixed and check which result repeats before changing the workflow.",
            "Different match counts in comparable replay conditions",
        ),
        "baseline_check" if detail["reason"] == "no_completion" => (
            "Check why the baseline runner failed",
            "Review the runner’s exit status and output limit. Repeat only the unchanged request until it returns a completed proposal.",
            "Every unchanged-request trial failed to return a completed proposal",
        ),
        "baseline_check" if detail["reason"] == "no_score" => (
            "Check the scoring rule on the original response",
            "Inspect the completed outputs and the configured match rule. Check that the expected field or text can be scored before testing source edits.",
            "The match rule could not score any completed unchanged-request trial",
        ),
        "baseline_check" => (
            "Check whether the runner can reproduce the recorded outcome",
            "Compare the original model, request template, tool parser and match rule. Run the unchanged request again before interpreting a source edit.",
            "None of the scored unchanged-request trials matched the recorded outcome",
        ),
        "source_gap" if detail["coverage"]["text_mismatches"].as_array().is_some_and(|ids| !ids.is_empty()) => (
            "Compare captured text with the upstream source",
            "Check the named fragment against the upstream value used to set its digest. Repeat preflight after correcting the fragment, its byte range or the expected value.",
            "The captured fragment differs from the workflow’s expected text",
        ),
        "source_gap" => (
            "Check the request builder and source capture",
            "Inspect the unconfirmed IDs for one alternative. Verify that the builder included those inputs and exported their named fragments, then repeat preflight.",
            "No configured alternative had all its required source fragments",
        ),
        "missing_tool_argument" if detail["check"]["other_declarations"].as_array().is_some_and(|rows| !rows.is_empty()) => (
            "Compare the two tool definitions",
            "Check the dispatcher response. To test whether the other definition influenced this proposal, rename its field in an isolated replay and compare with unchanged repeats.",
            "The proposal omitted a required field and supplied a name declared by another advertised tool.",
        ),
        "missing_tool_argument" => (
            "Check the proposed tool arguments",
            "Compare the proposed fields with the advertised schema. Inspect the dispatcher response to see whether it rejected or repaired the proposal.",
            "An argument object omitted a field required by the tool definition in the same request.",
        ),
        "example_citation" => (
            "Check where this citation points",
            "Compare the cited key with the labeled passages. Check whether the application displays or removes it, then test a citation example drawn from the current passages.",
            "The same key appeared in a citation-format example and a parenthetical in the model output, with no matching passage label in the captured messages.",
        ),
        "capability_claim" => (
            "Check what the discovery tool could have shown",
            "Compare the user's request with the quoted tool description. Check what the connected resource and its permissions allowed at that time.",
            "A recorded reply described a tool limitation. The request advertised a discovery function and the captured output contains only text.",
        ),
        "tool_reference" => (
            "Inspect how the application builds the tool history",
            "Compare the tool message at the recorded position with the preceding assistant invocations. Check where the application sets and preserves the invocation ID.",
            "Recorded tool message without a unique earlier invocation ID in this request",
        ),
        "conflict" => (
            "Resolve the competing rules",
            "Confirm which source should govern this scope. Test changing only the other declaration against the answer or tool choice you expect.",
            "Observed input disagreement",
        ),
        "duplicate" if detail["dialogue_only"] == true => (
            "Check whether the repetition belongs to the conversation",
            "Compare the message positions. Repeated replies can be part of a normal exchange. Test deduplication only if capture shows the workflow inserted an extra copy.",
            "Observed repeated dialogue without named sources",
        ),
        "duplicate" => (
            "Test removing one copy",
            "Keep one copy in its original role and test removing the other. Compare the named outcome before trimming the input in production.",
            "Observed exact repetition",
        ),
        "candidate" if ["baseline_outside_pair", "edited_outside_pair"].iter()
            .any(|key| detail[key].as_f64().is_some_and(|value| value > 0.5)) => (
            "Check which other answers are plausible",
            "Most probability fell outside the two tested continuations in at least one condition. Generate the full answer or include another plausible continuation in a follow-up test.",
            "Limited candidate coverage at the tested prefix",
        ),
        "candidate" => (
            "Check the edited answer or tool call",
            "Repeat the comparison at the same prefix and include an unrelated control edit. Check the full proposed answer or tool arguments before adopting the change.",
            "Measured preference at a fixed prefix",
        ),
        "prefill" => (
            "Compare the generated answer or tool call",
            "Replay the original and edited requests on the same runner. Check whether the measured first-token shift carries through to a completed proposal.",
            "Measured first-token distributions under named edits",
        ),
        "shadow" if detail["sandbox_dispatch"]["outcome"] == "matched_without_check" => (
            "Check why dispatch missed the expected state",
            "Review the approved proposal, checkpoint and guard condition. Repeat this comparison with a valid authorized request before enabling the check.",
            "Only the run without the dispatch check matched the expected sandbox state",
        ),
        "shadow" if detail["sandbox_dispatch"]["protected"] == true => (
            "Test the dispatch check on authorized requests",
            "The check blocked this proposal and preserved the expected state in the sandbox. Replay a valid approved proposal as a control before enabling the check.",
            "Same proposal and sandbox checkpoint with and without the dispatch check",
        ),
        "shadow" if detail["response_pair"]["changed"] == false && detail["measurement"]["baseline"]["matched"] == 0 => (
            "Test another source on this request",
            "The original and edited proposals were identical and both missed the expected outcome. Try a different source while keeping the request and scoring rule fixed.",
            "Matching complete proposals that missed the expected outcome",
        ),
        "shadow" if detail["response_pair"]["changed"] == false => (
            "Try the edit on another request",
            "Both conditions produced the same proposal here. Check a different input before drawing a conclusion about this source across the workflow.",
            "Matching complete proposals under the tested edit",
        ),
        "shadow" if detail["effect_pp"] == 0.0 && detail["response_pair"].is_null() => (
            "Inspect the individual trials",
            "The match rates were equal. The trials did not establish one consistent proposal in each condition.",
            "Equal match rates without a consistent proposal pair",
        ),
        "shadow" => (
            "Verify the edited proposal",
            "Check the complete proposed answer or tool arguments on more representative inputs before adopting the edit.",
            "Measured proposal rate",
        ),
        "token" => (
            "Investigate the source at this output step",
            "Repeat this intervention with neighboring tokens and a control source. Test fresh continuations to see whether the change carries forward.",
            "Measured token scores on a fixed output path",
        ),
        "source_mask" => (
            "Compare proposals after the source mask",
            "Check the proposed tool and arguments against the workflow contract. Repeat on representative inputs before changing a tool definition.",
            "Instrumented greedy continuations with an all-visible control",
        ),
        _ => (
            "Check whether the edit generalizes",
            "Repeat this edit on other requests in the same scope. Keep the outcome rule and runner configuration comparable.",
            "Measured replay outcome",
        ),
    };
    Action { title, next, basis }
}

pub(super) fn joint_followup() -> Action {
    Action {
        title: "Compare the four-condition results",
        next: "Check the linked examples for different proposals or unresolved trials before applying the result across conversations.",
        basis: "Comparable joint edits had more than one observed pattern",
    }
}

pub(super) fn prefill_action(card: &Value) -> Action {
    if let Some(pattern) = card
        .pointer("/evidence/detail/routing_test/pattern")
        .and_then(Value::as_str)
    {
        let profiles = card["routing_profiles"]
            .as_array()
            .expect("scoped routing patterns");
        let tested: u64 = profiles
            .iter()
            .filter_map(|row| row["inputs"].as_u64())
            .sum();
        let mixed = profiles
            .iter()
            .any(|r| r["signature"] == "mixed" || r["tool_signature"] == "mixed");
        let partial = profiles
            .iter()
            .any(|r| r["signature"].as_str().is_some_and(|s| s.contains('?')));
        let repeated = profiles
            .iter()
            .any(|r| r["example_runs"].as_u64().unwrap_or(0) > 1);
        if mixed || tested > 1 || repeated {
            let (title, next) = if mixed {
                ("Check the disagreeing repeat trials", "At least one repeated input gave different results under the same declared setup. Check the runner and repeat that input before drawing a conclusion.")
            } else if partial {
                ("Complete the missing routing trials", "Some tested inputs have incomplete comparisons. Fill the missing setups before applying the observed pattern across requests.")
            } else if tested == 1 {
                ("Test this edit on another input", "Completed repeat comparisons agreed for this input. Check the counts for each setup, then repeat the edit on a different request.")
            } else if profiles.len() > 1 {
                ("Compare the requests where the edit behaved differently", "Open an example from each result below. Check the other user messages and the proposed tool before changing the workflow.")
            } else {
                ("Validate the repeated routing pattern on another workflow", "The same four setups produced the same pattern on distinct requests. Test a different workflow before adopting the change more broadly.")
            };
            return Action {
                title,
                next,
                basis: "Distinct captured inputs with the same routing experiment and runner",
            };
        }
        let (title, next) = match pattern {
            "tools" => ("Retest the tool inventory on another request", "On the selected request, switching tool inventories changed whether the source edit affected the proposal under both instructions. Repeat the inventory change on a fresh input."),
            "instruction" => ("Retest the instruction on another request", "On the selected request, switching instruction text changed whether the source edit affected the proposal under both tool inventories. Repeat the instruction change on a fresh input."),
            "constant" => ("Test another source or workflow setting", "The edit had the same changed-or-unchanged result in all four setups on the selected request. Compare the proposals before choosing another factor."),
            "combined" => ("Test the instruction and tools together", "The edit had different effects across the four setups on the selected request. Repeat those combinations on a fresh input."),
            _ => ("Complete the missing routing trials", "At least one setup lacks a completed proposal. Repeat those trials before attributing the difference to either factor."),
        };
        return Action {
            title,
            next,
            basis: "One selected request, two instructions and two tool inventories",
        };
    }
    let runners = card["prefill_followup"]
        .as_array()
        .expect("scoped proposal counts");
    let ranks = card["prefill_counts"]
        .as_array()
        .expect("scoped first-token counts");
    let eligible: u64 = ranks
        .iter()
        .filter(|r| r["leader"]["status"] == "unique")
        .filter_map(|r| r["inputs"].as_u64())
        .sum();
    let (title,next) = match runners.as_slice() {
        [] if eligible == 0 => ("Check the edits along the output", "These inputs have no consistent unique leader at the first token. Test the source edits at a later token or compare complete proposals."),
        [] => ("Replay the leading edit", "Compare the full baseline and edited proposals under the same runner and settings."),
        [one] => {
            let changed = one["changed"].as_u64().unwrap_or(0);
            let equal = one["equal"].as_u64().unwrap_or(0);
            let mixed = one["mixed"].as_u64().unwrap_or(0);
            if changed+equal+mixed < eligible {
                ("Test the remaining inputs", "Some inputs have a leading edit and no completed proposal comparison for this runner. Start with one of those inputs.")
            } else if mixed>0 || (changed>0 && equal>0) {
                ("Compare where the edit mattered", "Check the captured instructions and tool inventories where the same edit led to different proposal effects.")
            } else if equal>0 {
                ("Check a later output token", "The leading edit shifted the first-token distribution while the completed proposals stayed the same. Inspect a later token or test another edit before changing the workflow.")
            } else {
                ("Repeat the leading edit on a fresh request", "The completed proposals changed on the tested inputs. Check the same edit on another conversation before revising the source.")
            }
        },
        _ => ("Compare the proposal runners", "Inspect each runner’s paired outcomes before applying the recommendation across runtimes."),
    };
    Action {
        title,
        next,
        basis: "First-token measurements and completed paired proposals in this scope",
    }
}

pub fn preview(
    store: &super::store::Store,
    input: pending::Request,
) -> Result<Value, (u16, String)> {
    let history_scope = super::history::Scope::read(&input.request);
    let pending::Inspected {
        mut call,
        scope,
        mut prefill,
        render_check,
        tool_scope,
        preflight,
        ..
    } = pending::preview(input).map_err(|message| (400, message))?;
    let mut history = match history_scope {
        Ok(history_scope) => history_scope
            .lookup(store, &call, scope.as_ref())
            .map_err(|e| (503, e.to_string()))?,
        Err(status) => json!({"status":status,"edits":[]}),
    };
    // The current pending contract determines this preview, even if the input
    // also carries a receipt from an earlier inspection.
    call.preflight_receipt = None;
    let run = Run {
        schema_version: a2context::SCHEMA_VERSION,
        id: "pending".into(),
        source: "pending".into(),
        organization_id: None,
        user_id: None,
        conversation_id: None,
        calls: vec![call],
        spans: vec![],
    };
    let call = &run.calls[0];
    let mut observations = findings::derive(&run, &[], &[]).map_err(|message| (400, message))?;
    observations.retain(|item| {
        item.kind != "conflict"
            || scope.as_ref().is_none_or(|scope| {
                item.detail["rule"].as_str() == Some(&scope.key)
                    && item.detail["scope"].as_str() == Some(&scope.scope)
            })
    });
    if let Some(coverage) = &preflight.source_coverage {
        observations.extend(findings::source_gap(&run, call, coverage));
    }
    observations.sort_by_key(|item| match item.kind {
        "source_gap" => 0,
        "shadow"
            if item.detail["effect_pp"] == 0.0
                && item.detail["response_pair"]["changed"] != true =>
        {
            4
        }
        "candidate" | "shadow" => 1,
        "conflict" => 2,
        _ => 3,
    });
    let scope_issue = tool_scope.as_ref().and_then(|check| {
        if !check.outside.is_empty() {
            Some(("Review tools outside the workflow scope", "Test the full and scoped catalogs on the same requests before changing what the model sees. Check permission again before dispatch."))
        } else if check.captured.is_none() || !check.unclassified.is_empty() {
            Some(("Check tool catalog capture", "Supply the assembled tools array and identify opaque entries before counting catalog coverage."))
        } else {
            None
        }
    });
    let render_issue = render_check.as_ref().filter(|v| v["unchanged"].as_u64().unwrap_or(0) > 0).map(|_| {
        json!({"kind":"render_gap","id":"pending-render-gap","detail":{},
            "action":{"title":"Check fields that left the model input unchanged",
                "next":"Inspect how your chat template serializes these fields before sending the request."}})
    });
    let total = observations.len()
        + usize::from(scope_issue.is_some())
        + usize::from(render_issue.is_some());
    let issue = scope_issue.map(|(title, next)| {
        json!({"kind":"tool_scope", "id":"pending-tool-scope",
               "detail": {"outside_count": tool_scope.as_ref().map(|check| check.outside.len())},
               "action": {"title":title, "next":next, "basis":"Declared function scope and captured catalog"}})
    });
    let findings = observations.into_iter().map(|item| {
        json!({"kind":item.kind, "id":item.group, "detail":item.detail, "action":item.action})
    });
    let items: Vec<_> = render_issue
        .into_iter()
        .chain(issue)
        .chain(findings)
        .take(15)
        .collect();
    let shadow = replay::shadow::recorded("pending", call);
    let tool_map = tool_scope.as_ref().map(|catalog| {
        let trials: Vec<_> = shadow
            .iter()
            .filter_map(|probe| match &probe.verification {
                replay::shadow::Verification::Matched { measurement } => {
                    Some(measurement.tool_proposals())
                }
                replay::shadow::Verification::Unverified { .. } => None,
            })
            .collect();
        json!({"permitted":catalog.permitted, "trials":trials})
    });
    if let Some(experiment) = history
        .get_mut("experiment")
        .filter(|e| e["status"] == "ready")
    {
        super::history::current_tests(experiment, call, &shadow, prefill.as_ref());
    }
    if let Some(probe) = prefill.as_mut() {
        super::prefill::attach_opening_checks(probe, call, &shadow);
    }
    let target_checks = super::targets::checks(call, &shadow);
    let screen = super::capture::screen_candidate(call);
    let candidates = replay::candidates::recorded("pending", call);
    let prefix_reviews = replay::candidates::prefix_reviews(&candidates, &shadow);
    let paths: Vec<Value> = candidates
        .iter()
        .filter_map(|probe| {
            let replay::candidates::Verification::Matched { measurement } = &probe.verification
            else {
                return None;
            };
            let paths: Vec<_> = (0..2)
                .filter_map(|i| replay::candidates::continuation_path(measurement, i))
                .collect();
            (!paths.is_empty()).then(|| json!(paths))
        })
        .collect();
    let checked_sources = preflight
        .source_coverage
        .as_ref()
        .map_or_else(Vec::new, |coverage| {
            checked_source_fragments(call, coverage)
        });
    let unverified = candidates
        .into_iter()
        .filter_map(|probe| match probe.verification {
            replay::candidates::Verification::Unverified { issue } => Some(issue),
            _ => None,
        })
        .chain(
            shadow
                .into_iter()
                .filter_map(|probe| match probe.verification {
                    replay::shadow::Verification::Unverified { issue } => Some(issue),
                    _ => None,
                }),
        )
        .chain(screen.as_ref().err().cloned())
        .take(10)
        .collect::<Vec<_>>();
    Ok(json!({
        "screen_suggestion":screen.ok().flatten(), "target_checks":target_checks, "continuation_paths":paths, "prefix_reviews":prefix_reviews,
        "version": 1, "model": call.model, "captured_inputs": call.context.len(),
        "scope_coverage": preflight.scope_coverage, "source_coverage": preflight.source_coverage,
        "checked_source_fragments": checked_sources,
        "anchor_overlap": preflight.anchor_overlap, "tool_scope":tool_scope,"tool_map":tool_map,"render_check":render_check,
        "total": total, "recommendations": items, "unverified_probes": unverified, "history":history, "prefill":prefill,
    }))
}

pub fn probes(run: &Run, call: &Call, replays: &[replay::Replay], findings: &mut Vec<Observation>) {
    for probe in replay::candidates::recorded(&run.id, call) {
        let replay::candidates::Verification::Matched { mut measurement } = probe.verification
        else {
            continue;
        };
        // Log-odds changes are comparable within this prefix and candidate pair.
        let baseline = measurement.baseline_log_odds;
        let flips =
            |margin: f64| (baseline > 0.0 && margin < 0.0) || (baseline < 0.0 && margin > 0.0);
        measurement.effects.sort_by(|a, b| {
            flips(b.log_odds)
                .cmp(&flips(a.log_odds))
                .then_with(|| b.change_log_odds.abs().total_cmp(&a.change_log_odds.abs()))
                .then_with(|| a.context_id.cmp(b.context_id))
        });
        for effect in measurement
            .effects
            .iter()
            .filter(|effect| effect.change_log_odds.abs() > 1e-10)
        {
            let Some(item) = call
                .context
                .iter()
                .find(|item| item.id == effect.context_id)
            else {
                continue;
            };
            let key = json!([
                "candidate",
                measurement.scorer_id,
                measurement.model_id,
                measurement.model_revision,
                measurement.runtime,
                measurement.tokenizer_sha256,
                measurement.assistant_prefix,
                measurement.candidates,
                findings::identity(run, item),
                effect.operation,
                effect.replacement
            ]);
            findings.push(findings::occurrence("candidate", key, call, json!({
                "report_id": probe.report_id, "sources": [findings::source(item)], "alternatives": measurement.candidates,
                "baseline": measurement.baseline_log_odds, "edited": effect.log_odds, "change": effect.change_log_odds,
                "baseline_outside_pair":measurement.baseline_outside_pair_probability, "edited_outside_pair":effect.outside_pair_probability,
                "model": measurement.model_id, "revision": measurement.model_revision, "runtime": measurement.runtime,
                "prefix": measurement.assistant_prefix, "replacement": effect.replacement, "operation": effect.operation,
                "other_edits": measurement.effects.iter().filter(|other| other.context_id != effect.context_id).take(4)
                    .map(|other| json!({"source": other.source_id, "id": other.context_id, "change": other.change_log_odds}))
                    .collect::<Vec<_>>(),
            })));
        }
    }
    let shadow = replay::shadow::recorded(&run.id, call);
    findings.extend(super::prefill::observation(run, call, &shadow, replays));
    let reports = call
        .shadow_probes
        .as_ref()
        .and_then(|c| c.reports.as_deref())
        .unwrap_or_default();
    for (probe, report) in shadow.into_iter().zip(reports) {
        let replay::shadow::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        for (effect, arm) in measurement.effects.iter().zip(&report.effects) {
            let pair = super::outcomes::compare(&report.baseline, &arm.result.trials);
            let Some(change) = effect.difference_pp else {
                continue;
            };
            let Some(item) = call
                .context
                .iter()
                .find(|item| item.id == effect.context_id)
            else {
                continue;
            };
            let key = json!([
                "shadow",
                measurement.runner_id,
                call.model,
                measurement.expected,
                measurement.match_rule,
                measurement.sampling_design,
                findings::identity(run, item),
                effect.intervention.operation,
                effect.intervention.replacement
            ]);
            let mut detail = json!({
                "report_id": probe.report_id, "sources": [findings::source(item)], "outcome": measurement.expected,
                "effect_pp": change, "response_pair": pair, "edited_request_sha256": arm.request_sha256,
                "model": call.model, "runner": measurement.runner_id,
                "measurement": {"baseline": measurement.baseline, "variant": effect.summary, "interval": effect.difference_95_interval_pp},
                "replacement": effect.intervention.replacement, "operation": effect.intervention.operation,
            });
            if let Some(dispatch) = replay::shadow::dispatch::review(report, arm) {
                detail["sandbox_dispatch"] = dispatch;
            }
            findings.push(findings::occurrence("shadow", key, call, detail));
        }
    }
}

pub fn token_measurements(
    run: &Run,
    reports: &[activation::Report],
    findings: &mut Vec<Observation>,
) {
    for report in reports {
        let Some(landmark) = activation::output_scores::landmark(report) else {
            continue;
        };
        let Some(call) = run
            .calls
            .iter()
            .find(|call| call.id == report.location().call_id)
        else {
            continue;
        };
        let Some(item) = call
            .context
            .iter()
            .find(|item| item.id == landmark.context_id)
        else {
            continue;
        };
        let changed_sources = activation::output_scores::changed_output_sources(report);
        let held = activation::output_scores::unchanged_output_source(report)
            .map(|id| json!({"generation":id}));
        let changed: Vec<_> = changed_sources.into_iter()
            .filter_map(|id| call.context.iter().find(|item| item.id == id))
            .map(|item| {
                let sources = [findings::source(item)];
                json!({"id":findings::source_set_id(run,&sources,None),"sources":sources,"generation":item.id})
            }).collect();
        findings.push(findings::occurrence("token", json!(["token", run.id, call.id, report.id()]), call, json!({
            "report_id": report.id(), "sources": [findings::source(item)], "location": report.location(),
            "landmark": landmark, "changed_edits":changed, "held_witness":held,
        })));
    }
}

pub fn tool_mask_measurements(
    run: &Run,
    reports: &[activation::Report],
    findings: &mut Vec<Observation>,
) {
    for report in reports {
        if !matches!(report, activation::Report::Visibility(_))
            || report.validate(std::slice::from_ref(run)).is_err()
        {
            continue;
        }
        let location = report.location();
        let Some(call) = run.calls.iter().find(|call| call.id == location.call_id) else {
            continue;
        };
        for trial in activation::visibility::tool_trials::tool_trials(report) {
            let mut inputs = trial
                .context_ids
                .iter()
                .filter_map(|id| call.context.iter().find(|item| item.id == *id))
                .collect::<Vec<_>>();
            inputs.sort_by_key(|item| findings::identity(run, item));
            let key = json!([
                "source_mask",
                [
                    trial.model_id,
                    trial.model_revision,
                    trial.runtime,
                    trial.tokenizer_sha256
                ],
                trial.target,
                trial.start,
                trial.target_tokens,
                findings::digest(json!(trial.fixed_prefix).to_string()),
                trial.max_new_tokens,
                trial.original.label,
                inputs
                    .iter()
                    .map(|item| findings::identity(run, item))
                    .collect::<Vec<_>>()
            ]);
            let before = super::outcomes::preview(&trial.original);
            let after = trial.masked.as_ref().map(super::outcomes::preview);
            let pair = after.as_ref().map(|edited| {
                json!({"original":before,"edited":edited,
                "changed":trial.outcome != "same_proposal","baseline_trials":1,"edited_trials":1})
            });
            findings.push(findings::occurrence("source_mask", key, call, json!({
                "report_id":report.id(), "sources":inputs.iter().map(|item| findings::source(item)).collect::<Vec<_>>(),
                "response_pair":pair, "measurement":{"location":location},
                "mask":{"outcome":trial.outcome,"target":trial.target,"start":trial.start,
                    "original":before,"proposal":after,"log_probability_change":trial.log_probability_change,
                    "budget":trial.max_new_tokens,"model":trial.model_id,
                    "trace":run.id,"inspector_path":trial.inspector_path}
            })));
        }
    }
}
