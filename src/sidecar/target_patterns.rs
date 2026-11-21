//! A target miss requires scored trials in every condition of the declared experiment.
use super::{outcomes, position_plan::digest};
use a2context::{
    replay::{candidates, shadow, MatchRule, TargetStatus},
    Call,
};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS target_experiments (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, report TEXT NOT NULL, design TEXT NOT NULL,
    request_hash TEXT NOT NULL, result TEXT NOT NULL, summary TEXT NOT NULL, question TEXT,
    PRIMARY KEY(trace_id,call_id,report),
    FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS target_design ON target_experiments(design,result,trace_id,call_id);";

fn enclosing_role<'a>(body: &'a Value, mut pointer: &str) -> Option<&'a str> {
    while let Some((parent, _)) = pointer.rsplit_once('/') {
        if let Some(role) = body.pointer(parent)?.get("role").and_then(Value::as_str) {
            return Some(role);
        }
        pointer = parent;
    }
    None
}

// One label per wholly missed experiment. Actual suffixes can be inspected even
// when the producer supplies no logits. A claim about every condition needs
// a source-bound score for the original, each single edit and any joint edit.
fn miss_signal(
    report: &shadow::Report,
    trials: &shadow::Measurement,
    probes: &[candidates::RecordedProbe],
) -> &'static str {
    if super::targets::has_extended_target(report) {
        return "suffix";
    }
    let conditions = 1 + report.effects.len() + usize::from(report.pair.is_some());
    let mut signal = "unscored";
    for probe in probes {
        let candidates::Verification::Matched { measurement } = &probe.verification else {
            continue;
        };
        let Some(path) = candidates::target_path(measurement, trials) else {
            continue;
        };
        if path.conditions.len() != conditions {
            continue;
        }
        let found = if path.conditions.iter().all(|arm| arm.first_lower == Some(0)) {
            "opening"
        } else {
            "other"
        };
        if signal != "unscored" && signal != found {
            return "varied";
        }
        signal = found;
    }
    signal
}

pub(super) fn index(tx: &Transaction, trace: &str, call: &Call) -> rusqlite::Result<()> {
    let Some(reports) = call.shadow_probes.as_ref().and_then(|c| c.reports.as_ref()) else {
        return Ok(());
    };
    let Some(mut body) = call
        .recorded_request
        .as_ref()
        .and_then(|r| serde_json::from_str::<Value>(&r.body).ok())
    else {
        return Ok(());
    };
    // Preserve sampling options, tool definitions and other top-level settings.
    // Message content varies across the requests being compared.
    body.sort_all_objects();
    let request_hash = digest(&body.to_string());
    let Some(fields) = body.as_object() else {
        return Ok(());
    };
    let options: std::collections::BTreeMap<_, _> = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "messages" | "input" | "contents" | "prompt"))
        .collect();
    let settings = digest(&json!(options).to_string());
    let question = super::capture::last_user_text(call).map(|q| q.to_string());
    let mut insert =
        tx.prepare_cached("INSERT INTO target_experiments VALUES (?,?,?,?,?,?,?,?)")?;
    let mut candidates = None;
    for (probe, report) in shadow::recorded(trace, call).into_iter().zip(reports) {
        let shadow::Verification::Matched { measurement } = probe.verification else {
            continue;
        };
        let mut sources = Vec::new();
        let mut edits = Vec::new();
        for (effect, raw) in measurement.effects.iter().zip(&report.effects) {
            let Some(source) = effect.source_id.filter(|s| !s.trim().is_empty()) else {
                break;
            };
            let edit = &raw.result.intervention;
            let context = call
                .context
                .iter()
                .find(|item| item.id == effect.context_id)
                .expect("verified shadow source");
            let control = report
                .control_comparison
                .as_ref()
                .is_some_and(|c| c.control_context_id == edit.context_id);
            edits.push(json!([
                source,
                effect.source_version,
                context.kind,
                context.role,
                enclosing_role(&body, &edit.pointer),
                raw.result.captured_context_sha256,
                edit.pointer,
                edit.text_range,
                edit.operation,
                edit.replacement,
                control
            ]));
            sources.push(source);
        }
        if edits.len() != report.effects.len() {
            continue;
        }
        edits.sort_by_cached_key(Value::to_string);
        sources.sort_unstable();
        sources.dedup();
        let mut design = json!([
            settings,
            call.model,
            report.runner_id,
            report.expected,
            report.match_rule,
            report.sampling_design,
            report.baseline.len(),
            edits,
            report.pair.is_some()
        ]);
        design.sort_all_objects();
        let mut result = "missed";
        for counts in std::iter::once(measurement.baseline)
            .chain(measurement.effects.iter().map(|e| e.summary))
            .chain(measurement.pair.iter().map(|p| p.summary))
        {
            match counts.target_status() {
                TargetStatus::Incomplete => {
                    result = "incomplete";
                    break;
                }
                TargetStatus::Met | TargetStatus::Mixed => result = "reached",
                TargetStatus::Missed => {}
            }
        }
        let mut summary = json!({"sources":sources, "expected":outcomes::preview(measurement.expected),
            "rule":measurement.match_rule, "runner":report.runner_id});
        if result == "missed"
            && report.expected.kind == "model_output"
            && report
                .expected
                .outcome
                .as_str()
                .is_some_and(|s| !s.is_empty())
            && matches!(report.match_rule, MatchRule::Exact)
        {
            let probes = candidates.get_or_insert_with(|| candidates::recorded(trace, call));
            summary["miss_signal"] = miss_signal(report, &measurement, probes).into();
        }
        insert.execute(params![
            trace,
            call.id,
            report.id,
            digest(&design.to_string()),
            request_hash,
            result,
            summary.to_string(),
            question
        ])?;
    }
    Ok(())
}

pub(super) const SCOPED: &str = ", runs AS (
    SELECT x.*,json_extract(x.summary,'$.miss_signal') AS signal,t.thread_key,t.thread,t.org,t.user,c.started FROM target_experiments x
    JOIN selected t ON x.trace_id=t.id JOIN captured_calls c USING(trace_id,call_id)
    WHERE t.thread_key IS NOT NULL";

pub(super) const CONVERSATIONS: &str = "), conversations AS (
    SELECT design,thread_key,count(*) AS reports, max(started) AS latest,
        CASE WHEN count(signal)!=count(*) THEN NULL
             WHEN min(signal)=max(signal) THEN min(signal)
             WHEN sum(signal='unscored')>0 THEN 'unscored' ELSE 'varied' END AS signal,
        CASE WHEN sum(result='incomplete')>0 THEN 'incomplete'
             WHEN min(result)!=max(result) THEN 'varied' ELSE min(result) END AS outcome
    FROM runs GROUP BY design,thread_key)";

// Qualify recurrence across distinct identified conversations. Count distinct request
// bodies separately so replaying one input across sessions is visible.
pub(super) const GROUPS: &str = ", groups AS (
    SELECT design,sum(outcome='missed') AS missed,sum(outcome='reached') AS reached,
        sum(outcome='incomplete') AS incomplete,sum(outcome='varied') AS varied,
        count(*) AS conversations,max(latest) AS latest,
        json_object('opening',count(*) FILTER (WHERE signal='opening'),'suffix',count(*) FILTER (WHERE signal='suffix'),
        'other',count(*) FILTER (WHERE signal='other'),'varied',count(*) FILTER (WHERE signal='varied'),
        'unscored',count(*) FILTER (WHERE signal='unscored')) AS signals FROM conversations GROUP BY design
    HAVING sum(outcome='missed')>=CAST(? AS INTEGER)
    ORDER BY missed DESC, latest DESC,design LIMIT ? OFFSET ?)
    SELECT json_object('design',g.design,'missed',g.missed,'reached',g.reached,'varied',g.varied,
        'incomplete',g.incomplete,'conversations',g.conversations,
        'missed_inputs',count(DISTINCT CASE WHEN c.outcome='missed' THEN r.request_hash END),
        'miss_signals',json(g.signals),'setup',json_remove(min(r.summary),'$.miss_signal')) FROM groups g JOIN runs r USING(design)
    JOIN conversations c ON (c.design,c.thread_key)=(g.design,r.thread_key) GROUP BY g.design
    ORDER BY g.missed DESC,g.latest DESC,g.design";

pub(super) const EXAMPLES: &str = " SELECT json_object('thread',r.thread,'user',r.user,'org',r.org,
    'trace',r.trace_id,'call',r.call_id,'report',r.report,'started',r.started,'result',r.result,
    'conversation_result',c.outcome,'miss_signal',c.signal,'question',json(r.question))
    FROM conversations c JOIN runs r USING(design,thread_key) WHERE 1";
pub(super) const EXAMPLE_ORDER: &str = " ORDER BY CASE c.outcome WHEN 'missed' THEN 0 WHEN 'varied' THEN 1 WHEN 'incomplete' THEN 2 ELSE 3 END,
        c.latest DESC,c.thread_key,r.started DESC,r.trace_id,r.call_id,r.report LIMIT ? OFFSET ?";
