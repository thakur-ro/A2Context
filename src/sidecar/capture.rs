//! Captured calls form the population for conversation-level findings.
use super::store::at;
use nexusweave::{
    context_screening::{self, Assessment, Pair, Source, Suggestion},
    Availability, Call, ContextItem, Run,
};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS captured_calls (
    trace_id TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE,
    call_id TEXT NOT NULL, started INTEGER, with_text INTEGER NOT NULL, screen TEXT,
    PRIMARY KEY(trace_id,call_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS captured_calls_started ON captured_calls(started,trace_id);
CREATE TABLE IF NOT EXISTS screened_inputs (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, input_id TEXT NOT NULL, detail TEXT NOT NULL,
    PRIMARY KEY(trace_id,call_id,input_id), FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS screened_pairs (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    pattern TEXT NOT NULL, relation TEXT NOT NULL, left_id TEXT NOT NULL, right_id TEXT NOT NULL,
    PRIMARY KEY(trace_id,call_id,ordinal), FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS screened_pairs_pattern ON screened_pairs(pattern,relation,trace_id,call_id);";

// Keep one review candidate per call. The detailed inspector shows the full screen.
pub(super) fn screen_candidate(call: &Call) -> Result<Option<Value>, String> {
    let Some(captured) = &call.context_screening else {
        return Ok(None);
    };
    Ok(review_pair(call, captured.verified(call)?))
}

fn review_pair(call: &Call, report: &context_screening::Report) -> Option<Value> {
    let scorer = report.scorer.as_ref()?;
    let flagged = report
        .pairs
        .iter()
        .filter(|pair| semantic_relation(pair).is_some());
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
        sources.push(source_excerpt(call, source)?);
    }
    Some(
        json!({"last_user_text":last_user_text(call),"suggestion":pair.suggestion,"sources":sources,"scorer":scorer,
        "left_to_right":left_to_right,"right_to_left":right_to_left,"flagged_pairs":count,"examined_pairs":report.pairs.len(),
        "total_pairs":report.total_pairs,"decision":call.decisions.iter().find(|d| d.kind!="tool_execution" && !d.outcome.is_null()).map(|d| &d.id)}),
    )
}

pub(super) fn last_user_message(call: &Call) -> Option<&ContextItem> {
    call.context
        .iter()
        .filter(|item| {
            item.role.as_deref() == Some("user") && matches!(item.kind.as_str(), "message" | "text")
        })
        .filter_map(|item| {
            // LangSmith keeps the message position in the recorded field path.
            let index = item.evidence.message_index.or_else(|| {
                item.evidence
                    .field
                    .strip_prefix("inputs.messages[")?
                    .split_once(']')?
                    .0
                    .parse()
                    .ok()
            })?;
            Some(((index, item.evidence.part_index), item))
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, item)| item)
}

pub(super) fn last_user_text(call: &Call) -> Option<Value> {
    last_user_message(call)
        .filter(|item| item.availability == Availability::Included)
        .and_then(|item| item.text.as_deref())
        .map(|s| {
            let mut chars = s.chars();
            let text: String = chars.by_ref().take(200).collect();
            json!({"text":text,"truncated":chars.next().is_some()})
        })
}

fn semantic_relation(pair: &Pair) -> Option<&'static str> {
    match pair.suggestion {
        Some(Suggestion::PossibleConflict) => Some("possible_conflict"),
        Some(Suggestion::PossibleOverlap) => Some("possible_overlap"),
        _ => None,
    }
}

fn source_excerpt(call: &Call, source: &Source) -> Option<Value> {
    let item = call
        .context
        .iter()
        .find(|item| item.id == source.context_id)?;
    let mut chars = item.text.as_deref()?.chars();
    let excerpt: String = chars.by_ref().take(200).collect();
    Some(
        json!({"id":source.context_id, "source":source.source_id, "version":source.source_version,
        "role":source.role, "digest":source.text_sha256, "text":excerpt, "truncated":chars.next().is_some()}),
    )
}

fn index_pairs(
    tx: &Transaction<'_>,
    trace: &str,
    call: &Call,
    report: &context_screening::Report,
) -> rusqlite::Result<()> {
    let Some(scorer) = &report.scorer else {
        return Ok(());
    };
    let mut inputs = std::collections::BTreeSet::new();
    let mut pairs = tx.prepare_cached("INSERT INTO screened_pairs VALUES (?,?,?,?,?,?,?)")?;
    for (ordinal, pair) in report.pairs.iter().enumerate() {
        let Some(relation) = semantic_relation(pair) else {
            continue;
        };
        inputs.extend(pair.sources);
        let sources = pair.sources.map(|index| &report.sources[index]);
        let pattern = context_screening::patterns::scored_pair_id(sources, scorer);
        pairs.execute(params![
            trace,
            call.id,
            ordinal,
            pattern,
            relation,
            sources[0].context_id,
            sources[1].context_id
        ])?;
    }
    let mut insert = tx.prepare_cached("INSERT INTO screened_inputs VALUES (?,?,?,?)")?;
    for index in inputs {
        let source = &report.sources[index];
        let detail = source_excerpt(call, source).expect("validated screen has captured text");
        insert.execute(params![
            trace,
            call.id,
            source.context_id,
            detail.to_string()
        ])?;
    }
    Ok(())
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
            let report = call
                .context_screening
                .as_ref()
                .and_then(|r| r.verified(call).ok());
            insert.execute(params![
                id,
                call.id,
                at(call.started_at.as_deref()),
                with_text,
                report
                    .and_then(|r| review_pair(call, r))
                    .map(|s| s.to_string())
            ])?;
            super::target_patterns::index(tx, id, call)?;
            if let Some(report) = report {
                index_pairs(tx, id, call, report)?;
            }
        }
        super::conversation::index(tx, run)?;
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
