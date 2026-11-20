//! Captured calls form the population for conversation-level findings.
use super::store::at;
use nexusweave::{
    context_screening::{self, Assessment, Suggestion},
    Availability, Call, Run,
};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS captured_calls (
    trace_id TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE,
    call_id TEXT NOT NULL, started INTEGER, with_text INTEGER NOT NULL, screen TEXT,
    PRIMARY KEY(trace_id,call_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS captured_calls_started ON captured_calls(started,trace_id);";

// Keep one review candidate per call. The detailed inspector shows the full screen.
pub(super) fn screen_candidate(call: &Call) -> Result<Option<Value>, String> {
    let Some(captured) = &call.context_screening else {
        return Ok(None);
    };
    Ok(review_pair(call, captured.verified(call)?))
}

fn review_pair(call: &Call, report: &context_screening::Report) -> Option<Value> {
    let scorer = report.scorer.as_ref()?;
    let flagged = report.pairs.iter().filter(|p| {
        matches!(
            p.suggestion,
            Some(Suggestion::PossibleConflict | Suggestion::PossibleOverlap)
        )
    });
    let count = flagged.clone().count();
    let pair = flagged.min_by_key(|p| p.suggestion != Some(Suggestion::PossibleConflict))?;
    let Assessment::Scored {
        left_to_right,
        right_to_left,
    } = pair.assessment
    else {
        return None;
    };
    let mut sources = Vec::with_capacity(2);
    for index in pair.sources {
        let source = &report.sources[index];
        let text = call
            .context
            .iter()
            .find(|i| i.id == source.context_id)?
            .text
            .as_deref()?;
        let mut chars = text.chars();
        let excerpt: String = chars.by_ref().take(200).collect();
        sources.push(
            json!({"id":source.context_id,"source":source.source_id,"version":source.source_version,
            "digest":source.text_sha256,"text":excerpt,"truncated":chars.next().is_some()}),
        );
    }
    Some(
        json!({"suggestion":pair.suggestion,"sources":sources,"scorer":scorer,
        "left_to_right":left_to_right,"right_to_left":right_to_left,"flagged_pairs":count,"examined_pairs":report.pairs.len(),
        "total_pairs":report.total_pairs,"decision":call.decisions.iter().find(|d| d.kind!="tool_execution" && !d.outcome.is_null()).map(|d| &d.id)}),
    )
}

pub(super) fn replace(tx: &Transaction<'_>, id: &str, run: Option<&Run>) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM captured_calls WHERE trace_id=?1", [id])?;
    if let Some(run) = run {
        let mut insert = tx.prepare_cached("INSERT INTO captured_calls VALUES (?1,?2,?3,?4,?5)")?;
        for call in &run.calls {
            let with_text = call.context.iter().any(|item| {
                item.availability == Availability::Included
                    && item.text.as_ref().is_some_and(|text| !text.is_empty())
            });
            insert.execute(params![
                id,
                call.id,
                at(call.started_at.as_deref()),
                with_text,
                screen_candidate(call).ok().flatten().map(|s| s.to_string())
            ])?;
        }
    }
    Ok(())
}

pub(super) const BY_CONVERSATION: &str = "), captured_threads AS (
    SELECT thread_key,count(*) AS calls,sum(with_text) AS with_text,max(started) AS latest,max(flagged) AS flagged
    FROM scoped_calls GROUP BY thread_key
), without_findings AS (
    SELECT ct.* FROM captured_threads ct WHERE NOT EXISTS (
        SELECT 1 FROM by_thread b WHERE b.thread_key=ct.thread_key)
)";

pub(super) const PAGE: &str = ", page AS (
    SELECT * FROM without_findings ORDER BY flagged DESC,(with_text>0),latest DESC,thread_key LIMIT ? OFFSET ?
), examples AS (
    SELECT c.*,row_number() OVER (PARTITION BY c.thread_key ORDER BY c.flagged DESC,c.with_text DESC,c.started DESC,c.trace_id,c.call_id) AS choice
    FROM page p JOIN scoped_calls c ON p.thread_key=c.thread_key
) SELECT json_object('thread',t.thread,'org',t.org,'user',t.user,'calls',p.calls,'with_text',p.with_text,
    'latest',p.latest,'trace',e.trace_id,'call',e.call_id,'screen',json(cc.screen))
    FROM page p JOIN examples e ON e.thread_key=p.thread_key AND e.choice=1 JOIN traces t ON t.id=e.trace_id
    JOIN captured_calls cc ON (cc.trace_id,cc.call_id)=(e.trace_id,e.call_id)
    ORDER BY p.flagged DESC,(p.with_text>0),p.latest DESC,p.thread_key";
