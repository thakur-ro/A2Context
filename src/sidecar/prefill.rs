//! Group comparable first-token tests and link any verified single-trial proposal checks.
use super::findings;
use nexusweave::{replay, Call, Run};
use serde_json::{json, Value};
mod rank_review;
mod reference;
mod token_contrast;
use reference::Reference;

pub(super) fn observation(
    run: &Run,
    call: &Call,
    shadow: &[replay::shadow::RecordedProbe],
    replays: &[replay::Replay],
) -> Option<findings::Observation> {
    let mut measured = replay::prefill::recorded(call, true)?.ok()?;
    let probe = call.prefill_probe.as_ref()?.report.as_ref()?;
    let reference = probe
        .answer_reference
        .as_ref()
        .map(Reference::parse)
        .transpose();
    if let Err(issue) = &reference {
        measured["reference_issue"] = json!(issue);
    }
    let reference = reference.ok().flatten();
    let mut design = Vec::new();
    let mut sources = Vec::new();
    for edit in &probe.interventions {
        let item = call.context.iter().find(|i| i.id == edit.context_id)?;
        design.push(
            json!([
                findings::identity(run, item),
                edit.pointer,
                edit.text_range,
                edit.operation,
                item.text,
                edit.replacement
            ])
            .to_string(),
        );
        sources.push(findings::source(item));
    }
    design.sort_unstable();
    let settings = settings(call)?;
    if let Some(tools) = settings["tools"].as_array() {
        measured["tool_count"] = json!(tools.len());
        measured["tool_names"] = json!(tools
            .iter()
            .take(8)
            .filter_map(|tool| tool
                .pointer("/function/name")
                .or_else(|| tool.get("name"))
                .and_then(Value::as_str))
            .collect::<Vec<_>>());
    }
    let key = json!([
        "prefill",
        run.source,
        run.organization_id,
        settings,
        measured["model_id"],
        measured["model_revision"],
        measured["runtime"],
        measured["tokenizer_sha256"],
        design
    ]);
    let ranked = measured["ranked"].as_array()?;
    let top = ranked.first()?;
    let shifted = top["js_bits"].as_f64()? > 0.0;
    let tied = ranked
        .get(1)
        .is_some_and(|r| r["js_bits"] == top["js_bits"]);
    let leader = if !shifted {
        json!({"status":"zero"})
    } else if tied {
        json!({"status":"tie"})
    } else {
        json!({"status":"unique","source":top["source"].as_str().or(top["context_id"].as_str()),
               "version":top["version"],"pointer":top["pointer"],"text_range":top["text_range"]})
    };
    let mut checks = proposal_checks(
        shadow,
        call,
        ranked,
        reference.as_ref(),
        probe.token_contrast.is_some(),
    );
    for report in replays.iter().filter(|r| {
        r.location.run_id == run.id
            && r.location.call_id == call.id
            && matches!(r.recorded.kind.as_str(), "model_output" | "tool_call")
            && r.rescored_from.is_none()
            && measured["baseline_request_sha256"] == r.baseline_request_sha256
    }) {
        let [replay::Trial::Completed { choice: original }] = report.baseline.as_slice() else {
            continue;
        };
        for variant in &report.variants {
            let [replay::Trial::Completed { choice: edited }] = variant.trials.as_slice() else {
                continue;
            };
            if probe.interventions.contains(&variant.intervention) {
                let mut check = json!({"context_id":variant.intervention.context_id,"report":report.id,
                    "runner":report.runner_id,"decision":report.location.decision_id,"changed":original!=edited,
                    "original":super::outcomes::preview(original),"edited":super::outcomes::preview(edited)});
                score_text_outputs(
                    &mut check,
                    reference.as_ref(),
                    probe.token_contrast.is_some(),
                    original,
                    edited,
                );
                checks.push(check);
            }
        }
    }
    if probe.interventions.len() >= 2 {
        let first = &probe.interventions[0].context_id;
        let top_id = top["context_id"].as_str()?;
        let (behavior, grades) = paired_result(&checks, first, top_id);
        let mut comparison = json!({"first":first, "same_edit":first==top_id,"result":behavior});
        comparison["rank_review"] = rank_review::review(ranked, &checks);
        if let Some(raw) = &probe.token_contrast {
            match token_contrast::checked(raw, &measured, &comparison["rank_review"], &checks) {
                Ok(readout) => comparison["token_contrast"] = readout,
                Err(issue) => comparison["token_contrast_issue"] = json!(issue),
            }
        }
        if reference.is_some() {
            comparison["reference"] = grades;
            measured["answer_reference"] = json!(probe.answer_reference);
        }
        measured["selector_check"] = comparison;
    }
    measured["sources"] = json!(sources);
    measured["leader"] = json!(leader);
    measured["shifted"] = json!(shifted);
    measured["proposal_checks"] = json!(checks);
    Some(findings::occurrence("prefill", key, call, measured))
}

fn score_text_outputs(
    check: &mut Value,
    reference: Option<&Reference>,
    with_digests: bool,
    original: &replay::Choice,
    edited: &replay::Choice,
) {
    if with_digests && original.kind == "model_output" && edited.kind == "model_output" {
        if let Some((a, b)) = original.outcome.as_str().zip(edited.outcome.as_str()) {
            check["answer_digests"] = json!([findings::digest(a), findings::digest(b)]);
        }
    }
    if let Some(r) = reference {
        if let Some(pair) = r.score(original).zip(r.score(edited)) {
            check["reference_match"] = json!(pair);
        }
    }
}

fn paired_result(checks: &[Value], first: &str, top: &str) -> (&'static str, Value) {
    fn report_key(check: &Value) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            check["runner"].as_str(),
            check["report"].as_str(),
            check["decision"].as_str(),
        )
    }
    let first_by_report: std::collections::BTreeMap<_, _> = checks
        .iter()
        .filter(|c| c["context_id"] == first)
        .map(|c| (report_key(c), c))
        .collect();
    let mut result = "missing";
    let mut grades = std::collections::BTreeSet::new();
    for ranked in checks.iter().filter(|c| c["context_id"] == top) {
        let Some(first) = first_by_report.get(&report_key(ranked)) else {
            continue;
        };
        let a = &ranked["reference_match"];
        let b = &first["reference_match"];
        if let (Some(original), Some(top), Some(first)) =
            (a[0].as_bool(), a[1].as_bool(), b[1].as_bool())
        {
            if b[0].as_bool() == Some(original) {
                grades.insert((original, top, first));
            }
        }
        let pair = match (ranked["changed"] == true, first["changed"] == true) {
            (true, true) => "both",
            (true, false) => "ranked_only",
            (false, true) => "first_only",
            (false, false) => "neither",
        };
        if result != "missing" && result != pair {
            result = "mixed";
        } else {
            result = pair;
        }
    }
    let reference = match grades.len() {
        0 => json!({"status":"missing"}),
        1 => {
            let (original, ranked, first) = grades.into_iter().next().unwrap();
            json!({"status":"scored","original":original,"ranked":ranked,"first":first})
        }
        _ => json!({"status":"mixed"}),
    };
    (result, reference)
}

// Keep system instructions, tool inventories, message positions and request settings fixed.
fn settings(call: &Call) -> Option<Value> {
    let mut settings: Value = serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
    for field in ["messages", "input"] {
        if field == "input" && settings[field].is_string() {
            settings[field] = Value::Null;
        }
        if let Some(messages) = settings[field].as_array_mut() {
            for message in messages.iter_mut().filter(|m| m["role"] == "user") {
                if message["content"].is_string() {
                    message["content"] = Value::Null;
                } else if let Some(parts) = message["content"].as_array_mut() {
                    for part in parts.iter_mut().filter(|p| {
                        matches!(p["type"].as_str(), Some("input_text" | "text"))
                            && p["text"].is_string()
                    }) {
                        part["text"] = Value::Null;
                    }
                }
            }
        }
    }
    settings.sort_all_objects();
    Some(settings)
}

pub(super) fn attach_opening_checks(
    probe: &mut Value,
    call: &Call,
    shadow: &[replay::shadow::RecordedProbe],
) {
    let Some(rows) = probe["ranked"].as_array().filter(|rows| {
        rows.first()
            .is_some_and(|row| row["opening_changed"].is_boolean())
    }) else {
        return;
    };
    let mut checks = Vec::new();
    matching_shadow_arms(shadow, call, rows, |report, effect, id| {
        let mut check = super::outcomes::compare(&report.baseline, &effect.result.trials)
            .unwrap_or_else(|| {
                let complete = report.baseline_summary.completed == report.baseline.len()
                    && effect.result.summary.completed == effect.result.trials.len();
                json!({"status":if complete { "varied" } else { "incomplete" }})
            });
        check["context_id"] = json!(id);
        check["runner"] = json!(&report.runner_id);
        check["report"] = json!(&report.id);
        checks.push(check);
    });
    probe["proposal_checks"] = json!(checks);
}

fn proposal_checks(
    shadow: &[replay::shadow::RecordedProbe],
    call: &Call,
    ranked: &[Value],
    reference: Option<&Reference>,
    with_digests: bool,
) -> Vec<Value> {
    let mut checks = Vec::new();
    matching_shadow_arms(shadow, call, ranked, |report, effect, id| {
        if let (
            [replay::Trial::Completed { choice: original }],
            [replay::Trial::Completed { choice: edited }],
        ) = (report.baseline.as_slice(), effect.result.trials.as_slice())
        {
            let mut check = json!({"context_id":id,"report":report.id,"runner":report.runner_id,
                "changed":original!=edited,"original":super::outcomes::preview(original), "edited":super::outcomes::preview(edited)});
            score_text_outputs(&mut check, reference, with_digests, original, edited);
            checks.push(check);
        }
    });
    checks
}

fn matching_shadow_arms(
    shadow: &[replay::shadow::RecordedProbe],
    call: &Call,
    ranked: &[Value],
    mut visit: impl FnMut(&replay::shadow::Report, &replay::shadow::Effect, &str),
) {
    let Some(reports) = call.shadow_probes.as_ref().and_then(|c| c.reports.as_ref()) else {
        return;
    };
    for (verified, report) in shadow.iter().zip(reports) {
        let replay::shadow::Verification::Matched { measurement } = &verified.verification else {
            continue;
        };
        for (effect, resolved) in report.effects.iter().zip(&measurement.effects) {
            if ranked.iter().any(|row| {
                row["request_sha256"] == effect.request_sha256
                    && row["context_id"] == resolved.context_id
            }) {
                visit(report, effect, resolved.context_id);
            }
        }
    }
}

// Count each captured input once. Disagreeing repeats retain their own category.
pub(super) const SCOPED_COUNTS: &str = ", prefill_inputs AS (
                    SELECT e.group_id,json_extract(e.detail,'$.baseline_request_sha256') AS input,
                        CASE WHEN min(json_extract(e.detail,'$.leader'))=max(json_extract(e.detail,'$.leader'))
                            THEN min(json_extract(e.detail,'$.leader')) ELSE json_object('status','mixed') END AS leader
                    FROM groups g JOIN events e ON e.group_id=g.group_id WHERE g.kind='prefill'
                    GROUP BY e.group_id,input
                 ), prefill_counts AS (
                    SELECT group_id,leader,count(*) AS inputs FROM prefill_inputs GROUP BY group_id,leader
                 )
                 , prefill_replays AS (
                    SELECT p.group_id,p.input,json_extract(c.value,'$.runner') AS runner,
                        min(json_extract(c.value,'$.changed')) AS low,max(json_extract(c.value,'$.changed')) AS high
                    FROM prefill_inputs p JOIN events e ON e.group_id=p.group_id
                      AND json_extract(e.detail,'$.baseline_request_sha256')=p.input
                    JOIN json_each(e.detail,'$.proposal_checks') c
                    WHERE json_extract(p.leader,'$.status')='unique'
                      AND json_extract(c.value,'$.context_id')=json_extract(e.detail,'$.ranked[0].context_id')
                    GROUP BY p.group_id,p.input,runner
                 ) ";

// A missing pair cannot contradict a completed pair. Match opposite results in the same setup.
pub(super) fn append_routing(sql: &mut String, page_only: bool) {
    sql.push_str(", routing_events AS (
                    SELECT e.group_id,e.factor,e.thread_key,json_extract(e.detail,'$.routing_test.design') AS design,
                        json_extract(e.detail,'$.baseline_request_sha256') AS input,
                        json_extract(e.detail,'$.routing_test.signature') AS signature,
                        json_extract(e.detail,'$.routing_test.tool_signature') AS tool_signature,
                        json_object('trace',e.trace_id,'call',e.call_id) AS example
                    FROM events e WHERE e.kind='prefill' AND json_extract(e.detail,'$.routing_test.design') IS NOT NULL");
    if page_only {
        sql.push_str(" AND e.group_id IN (SELECT group_id FROM groups)");
    }
    sql.push_str("), routing_signatures AS (
                    SELECT group_id,design,input,min(factor) AS factor,signature,tool_signature,min(example) AS example,count(*) AS runs
                    FROM routing_events GROUP BY group_id,design,input,signature,tool_signature
                 ), routing_best AS (
                    SELECT group_id,design,input,min(factor) AS factor,sum(runs) AS report_count,
                        min(json_array(4-length(replace(signature,'?','')),json(example),signature,tool_signature))
                            AS sample
                    FROM routing_signatures GROUP BY group_id,design,input
                 ), setups(slot) AS (VALUES (0),(1),(2),(3)), metrics(metric) AS (VALUES ('proposal'),('tool')),
                 routing_slots AS (
                    SELECT s.group_id,s.design,s.input,metric,slot,s.example,s.runs,
                        substr(CASE metric WHEN 'tool' THEN tool_signature ELSE signature END,slot+1,1) AS outcome
                    FROM routing_signatures s JOIN routing_best b USING(group_id,design,input)
                    CROSS JOIN setups CROSS JOIN metrics WHERE b.report_count>1
                 ), routing_votes AS (
                    SELECT group_id,design,input,metric,slot,
                        min(example) FILTER (WHERE outcome='0') AS held,min(example) FILTER (WHERE outcome='1') AS changed,
                        sum(runs*(outcome='0')) AS held_runs,sum(runs*(outcome='1')) AS changed_runs,sum(runs*(outcome='?')) AS missing_runs
                    FROM routing_slots GROUP BY group_id,design,input,metric,slot
                 ), routing_conflicts AS (
                    SELECT group_id,design,input,json_group_object(metric,json(pair)) AS witnesses FROM (
                        SELECT group_id,design,input,metric,min(json_object('setup',slot,'held',json(held),'changed',json(changed),
                            'held_runs',held_runs,'changed_runs',changed_runs,'missing_runs',missing_runs)) AS pair
                        FROM routing_votes WHERE held IS NOT NULL AND changed IS NOT NULL GROUP BY group_id,design,input,metric
                    ) GROUP BY group_id,design,input
                 ), repeat_counts AS (
                    SELECT group_id,design,input,json_group_array(json_object('proposal',proposal,'tool',tool,'missing',missing)
                        ORDER BY slot) AS counts FROM (
                        SELECT group_id,design,input,slot,max(changed_runs) FILTER (WHERE metric='proposal') AS proposal,
                            max(changed_runs) FILTER (WHERE metric='tool') AS tool,max(missing_runs) AS missing
                        FROM routing_votes GROUP BY group_id,design,input,slot
                    ) GROUP BY group_id,design,input
                 ), routing_inputs AS (
                    SELECT group_id,design,input,factor,json_extract(sample,'$[1]') AS example,witnesses,counts,report_count,
                        CASE WHEN json_type(witnesses,'$.proposal') IS NULL THEN json_extract(sample,'$[2]') ELSE 'mixed' END AS signature,
                        CASE WHEN json_type(witnesses,'$.tool') IS NULL THEN json_extract(sample,'$[3]') ELSE 'mixed' END AS tool_signature
                    FROM routing_best LEFT JOIN routing_conflicts USING(group_id,design,input) LEFT JOIN repeat_counts USING(group_id,design,input)
                 ), routing_designs AS (
                    SELECT group_id,design,count(*) AS inputs,
                        sum(signature!='mixed' AND instr(signature,'?')=0) AS completed
                    FROM routing_inputs GROUP BY group_id,design)");
}

#[cfg(test)]
mod ranking_tests {
    use super::*;

    #[test]
    fn only_pairs_checks_from_the_same_runner_report_and_decision() {
        let first = json!({"runner":"q","report":"r","decision":"one","context_id":"first","changed":false});
        let top =
            json!({"runner":"q","report":"r","decision":"one","context_id":"top","changed":true});
        for key in ["runner", "report", "decision"] {
            let mut unrelated = top.clone();
            unrelated[key] = "different".into();
            assert_eq!(
                paired_result(&[first.clone(), unrelated], "first", "top").0,
                "missing"
            );
        }
        let mut next_first = first.clone();
        next_first["report"] = "second".into();
        let mut next_top = top.clone();
        next_top["report"] = "second".into();
        next_top["changed"] = false.into();
        assert_eq!(
            paired_result(&[first, top, next_first, next_top], "first", "top").0,
            "mixed"
        );
    }
}
