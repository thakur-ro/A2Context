//! Bounded dialogue previews and call-level finding links for a scoped thread.
use a2context::{Availability, Call, Run};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS conversation_calls (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, ordinal INTEGER NOT NULL, nanos INTEGER, preview TEXT NOT NULL,
    PRIMARY KEY(trace_id,call_id), FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS observations_by_call ON observations(trace_id,call_id);";

fn excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let first: String = chars.by_ref().take(240).collect();
    if chars.clone().take(121).count() <= 120 {
        return first + chars.as_str();
    }
    let tail: String = chars.rev().take(120).collect();
    first + " … " + &tail.chars().rev().collect::<String>()
}

pub(super) fn preview(call: &Call) -> Value {
    let outputs = call.decisions.iter().filter(|d| {
        matches!(
            d.kind.as_str(),
            "model_output" | "tool_call" | "tool_execution"
        )
    });
    let shown: Vec<_> = outputs
        .clone()
        .take(3)
        .map(|d| {
            let text = match &d.outcome {
                Value::Null => None,
                Value::String(s) => Some(excerpt(s)),
                other => Some(excerpt(&other.to_string())),
            };
            json!({"kind":d.kind,"label":excerpt(&d.label),"text":text})
        })
        .collect();
    let user = super::capture::last_user_message(call).map(|item| {
        let text = item
            .text
            .as_deref()
            .filter(|_| item.availability == Availability::Included);
        json!({"availability":item.availability, "text":text.map(excerpt),
            "truncated":text.is_some_and(|s| s.chars().nth(360).is_some())})
    });
    json!({"turn":call.turn_id,"model":call.model,
        "user":user,
        "outputs":shown,"output_count":outputs.count(),"inputs":call.context.len(),
        "included":call.context.iter().filter(|c| c.availability==Availability::Included).count()})
}

pub(super) fn index(tx: &Transaction<'_>, run: &Run) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO conversation_calls(trace_id,call_id,ordinal,nanos,preview)
        VALUES (?,?,?,?,?) ON CONFLICT(trace_id,call_id) DO UPDATE SET
        ordinal=excluded.ordinal,nanos=excluded.nanos,preview=excluded.preview",
    )?;
    for (ordinal, call) in run.calls.iter().enumerate() {
        let nanos = call
            .started_at
            .as_deref()
            .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
            .map(|t| t.nanosecond());
        let preview = preview(call);
        stmt.execute(params![
            run.id,
            call.id,
            ordinal,
            nanos,
            preview.to_string()
        ])?;
        super::activity_search::index(tx, &run.id, &call.id, &preview)?;
    }
    Ok(())
}

pub(super) fn needs_tool_preview(dialogue: &Value) -> bool {
    dialogue["output_count"] == 0 && dialogue["model"].is_null() && dialogue["inputs"] == 0
}

// Only inspect findings for the calls on this page. A source conflict stays independently
// discoverable even when a stronger, unrelated replay is attached to the same call.
pub(super) const START: &str = ", thread_calls AS (
    SELECT c.trace_id,c.call_id,c.started,d.ordinal,d.nanos,t.thread,d.preview FROM selected t
    JOIN captured_calls c ON c.trace_id=t.id JOIN conversation_calls d USING(trace_id,call_id) WHERE 1";

pub(super) const ORDER: &str = "started IS NULL,started,nanos,trace_id,ordinal";

pub(super) const PAGE: &str = " LIMIT ? OFFSET ?
), candidates AS (
    SELECT o.trace_id,o.call_id,o.kind,o.group_id,o.n,
        json_extract(o.detail,'$.selector_check.token_contrast.position') AS token_position,
        json_extract(o.detail,'$.selector_check.rank_review.status') AS review,
        json_array_length(o.detail,'$.changed_edits')>0 AS proposal_changed,
        json_extract(o.detail,'$.effect_pp') AS effect,
        json_extract(o.detail,'$.shifted') AS shifted,
        json_extract(o.detail,'$.measurement.replay_id') AS replay,
        json_extract(o.detail,'$.measurement.location.decision_id') AS decision,
        json_extract(o.detail,'$.measurement.context_id') AS context,
        json_extract(o.detail,'$.report_id') AS report,
        CASE WHEN json_extract(o.detail,'$.selector_check.rank_review.status')='lower_changed' THEN (
            SELECT coalesce(json_extract(r.value,'$.source'),json_extract(r.value,'$.context_id')) FROM json_each(o.detail,'$.ranked') r
            WHERE json_extract(r.value,'$.context_id')=json_extract(o.detail,'$.proposal_checks[' ||
                json_extract(o.detail,'$.selector_check.rank_review.witness.lower_check') || '].context_id') LIMIT 1)
        ELSE coalesce(json_extract(o.detail,'$.changed_edits[0].sources[0].source'),json_extract(o.detail,'$.sources[0].source')) END AS source,
        json_extract(o.detail,'$.sources[1].source') AS other_source
    FROM call_page p JOIN observations o ON (o.trace_id,o.call_id)=(p.trace_id,p.call_id)
), scored AS (
    SELECT *, CASE WHEN review='lower_changed' AND token_position IS NOT NULL THEN 0
        WHEN review='lower_changed' THEN 1 WHEN proposal_changed THEN 2
        WHEN kind='replacement' OR effect!=0 THEN 3
        WHEN kind IN ('candidate','token') OR shifted THEN 4 WHEN kind IN ('replay','shadow','prefill') THEN 5
        WHEN kind='conflict' THEN 6 ELSE 7 END AS priority FROM candidates
), ranked AS (
    SELECT *, row_number() OVER (PARTITION BY trace_id,call_id,kind ORDER BY priority,n) AS sample,
        count(*) OVER (PARTITION BY trace_id,call_id,kind) AS observations FROM scored
) SELECT json_object('trace',p.trace_id,'call',p.call_id,'thread',p.thread,'started',p.started,'nanos',p.nanos,'dialogue',json(p.preview),
    'findings',json((SELECT json_group_array(json_object('kind',kind,'group',group_id,'count',observations,
        'priority',priority,'source',source,'other_source',other_source,'token_position',token_position,
        'replay',replay,'decision',decision,'context',context,'report',report) ORDER BY priority,n)
        FROM ranked WHERE trace_id=p.trace_id AND call_id=p.call_id AND sample=1)))
    FROM call_page p ORDER BY p.started IS NULL,p.started,p.nanos,p.trace_id,p.ordinal";
