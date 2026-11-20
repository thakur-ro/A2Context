//! Group comparable first-token tests and link any verified single-trial proposal checks.
use super::findings;
use nexusweave::{replay, Call, Run};
use serde_json::{json, Value};

pub(super) fn observation(
    run: &Run,
    call: &Call,
    shadow: &[replay::shadow::RecordedProbe],
) -> Option<findings::Observation> {
    let mut measured = replay::prefill::recorded(call, true)?.ok()?;
    let probe = call.prefill_probe.as_ref()?.report.as_ref()?;
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
    let checks = proposal_checks(shadow, call, ranked);
    measured["sources"] = json!(sources);
    measured["leader"] = json!(leader);
    measured["shifted"] = json!(shifted);
    measured["proposal_checks"] = json!(checks);
    Some(findings::occurrence("prefill", key, call, measured))
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

fn proposal_checks(
    shadow: &[replay::shadow::RecordedProbe],
    call: &Call,
    ranked: &[Value],
) -> Vec<Value> {
    let mut checks = Vec::new();
    let Some(reports) = call.shadow_probes.as_ref().and_then(|c| c.reports.as_ref()) else {
        return checks;
    };
    for (verified, report) in shadow.iter().zip(reports) {
        let replay::shadow::Verification::Matched { measurement } = &verified.verification else {
            continue;
        };
        let [replay::Trial::Completed { choice: original }] = report.baseline.as_slice() else {
            continue;
        };
        for (effect, resolved) in report.effects.iter().zip(&measurement.effects) {
            let [replay::Trial::Completed { choice: edited }] = effect.result.trials.as_slice()
            else {
                continue;
            };
            if ranked.iter().any(|row| {
                row["request_sha256"] == effect.request_sha256
                    && row["context_id"] == resolved.context_id
            }) {
                checks.push(json!({"context_id":resolved.context_id,"report":report.id,"runner":report.runner_id,
                    "changed":original!=edited,"original":super::outcomes::preview(original), "edited":super::outcomes::preview(edited)}));
            }
        }
    }
    checks
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
