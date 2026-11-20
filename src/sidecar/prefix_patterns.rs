//! Index the prediction and its checked outcome without copying scores or trial text.
use super::findings::digest;
use nexusweave::{
    replay::{candidates, shadow},
    Call, Run,
};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS prefix_checks (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, report TEXT NOT NULL, design TEXT NOT NULL,
    prefix TEXT NOT NULL, source TEXT, verdict TEXT NOT NULL, input TEXT NOT NULL, setup TEXT, question_hash TEXT, choices TEXT NOT NULL,
    PRIMARY KEY(trace_id,call_id,report),
    FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS prefix_checks_design ON prefix_checks(design,verdict,source);";

pub(super) fn index(tx: &Transaction, run: &Run, call: &Call) -> rusqlite::Result<()> {
    let Some(reports) = call
        .candidate_probes
        .as_ref()
        .and_then(|p| p.reports.as_ref())
    else {
        return Ok(());
    };
    if !reports.iter().any(|r| !r.assistant_prefix.is_empty()) {
        return Ok(());
    }
    let candidates = candidates::recorded(&run.id, call);
    let trials = shadow::recorded(&run.id, call);
    let mut insert =
        tx.prepare_cached("INSERT INTO prefix_checks VALUES (?,?,?,?,?,?,?,?,?,?,?)")?;
    for (record, raw) in candidates.iter().zip(reports) {
        for r in candidates::prefix_reviews(std::slice::from_ref(record), &trials) {
            let selection =
                r["reversals"]
                    .as_array()
                    .and_then(|indices| match indices.as_slice() {
                        [one] => one.as_u64().map(|index| &r["conditions"][index as usize]),
                        _ => None,
                    });
            let design = digest(
                json!([
                    run.source,
                    run.organization_id,
                    r["model"],
                    r["revision"],
                    r["runtime"],
                    r["scorer"],
                    r["tokenizer_sha256"],
                    r["prefix"],
                    r["alternatives"]
                ])
                .to_string(),
            );
            let preferred = |c: &Value| {
                c["preferred"]
                    .as_u64()
                    .and_then(|i| r["alternatives"][i as usize].as_str())
            };
            let choices = json!({"alternatives":r["alternatives"],"model":r["model"],"revision":r["revision"],"runtime":r["runtime"],"scorer":r["scorer"],
                "original":preferred(&r["conditions"][0]),"edited":selection.and_then(preferred)});
            insert.execute(params![
                run.id,
                call.id,
                raw.id,
                design,
                raw.assistant_prefix,
                selection.and_then(|s| s["source"].as_str()),
                r["verdict"].as_str(),
                raw.captured_request_sha256,
                matched_setup(call, &r),
                super::capture::last_user_message(call)
                    .and_then(|c| c.text.as_deref())
                    .map(digest),
                choices.to_string()
            ])?;
        }
    }
    Ok(())
}

// Require one complete source-edit design with a single identified runner.
// Individual outcomes can still disagree or be unresolved under this design.
fn matched_setup(call: &Call, review: &Value) -> Option<String> {
    let [run] = review["runs"].as_array()?.as_slice() else {
        return None;
    };
    let conditions = review["conditions"].as_array()?;
    let rows = run["rows"].as_array()?;
    let mut edits = Vec::new();
    for (c, row) in conditions.iter().zip(rows).skip(1) {
        edits.push(
            json!([
                c["source"].as_str()?,
                c["version"],
                c["original_sha256"],
                c["edit"],
                row["attempted"].as_u64()?
            ])
            .to_string(),
        );
    }
    edits.sort_unstable();
    Some(digest(
        json!([
            super::conflict::question_setup(call)?,
            run["runner"],
            run["sampling"],
            rows.first()?["attempted"],
            edits
        ])
        .to_string(),
    ))
}

pub(super) const SCOPED: &str = ", checks AS (
    SELECT p.*,t.thread_key,t.thread,c.started FROM prefix_checks p JOIN selected t ON t.id=p.trace_id
    JOIN captured_calls c USING(trace_id,call_id) WHERE t.thread_key IS NOT NULL";

pub(super) const GROUPS: &str = " SELECT json_object('design',design,'prefix',prefix,
    'model',choices->>'model','revision',choices->>'revision','runtime',choices->>'runtime','scorer',choices->>'scorer','alternatives',choices->'alternatives',
    'conversations',count(DISTINCT thread_key),'inputs',count(DISTINCT input),
    'exceptions',count(DISTINCT thread_key) FILTER (WHERE verdict IN ('disagreed','unselected_change','mixed')),
    'confirmed',count(DISTINCT thread_key) FILTER (WHERE verdict='confirmed'),
    'untested',count(DISTINCT thread_key) FILTER (WHERE verdict='untested'))
    FROM checks GROUP BY design,prefix ORDER BY
    count(DISTINCT thread_key) FILTER (WHERE verdict IN ('disagreed','unselected_change','mixed')) DESC,
    count(DISTINCT thread_key) DESC,max(started) DESC,design LIMIT ? OFFSET ?";

pub(super) const BREAKDOWN: &str = " SELECT json_object('source',source,'verdict',verdict,
    'conversations',count(DISTINCT thread_key),'inputs',count(DISTINCT input))
    FROM checks GROUP BY source,verdict ORDER BY count(DISTINCT thread_key) DESC,source,verdict LIMIT ? OFFSET ?";

pub(super) const CASES: &str = ", witnesses AS (
    SELECT *,row_number() OVER(PARTITION BY thread_key ORDER BY started DESC,trace_id,call_id,report) AS sample
    FROM checks) SELECT json_object('thread',w.thread,'thread_key',w.thread_key,'trace',w.trace_id,'call',w.call_id,
    'report',w.report,'source',w.source,'verdict',w.verdict,'question',c.preview->>'$.user.text')
    FROM witnesses w JOIN conversation_calls c USING(trace_id,call_id) WHERE sample=1
    ORDER BY w.started DESC,w.thread_key LIMIT ? OFFSET ?";

// Keep one verified example per question, then one per source. A repeated
// question with incompatible results cannot supply either side of a contrast.
pub(super) const CONTRASTS: &str = ", comparable AS (
    SELECT *,row_number() OVER(PARTITION BY setup,question_hash ORDER BY started DESC,trace_id,call_id,report) AS witness,
        min(verdict) OVER w AS lo,max(verdict) OVER w AS hi,
        count(source) OVER w AS named,count(*) OVER w AS total,min(source) OVER w AS first_source,max(source) OVER w AS last_source
    FROM checks WHERE setup IS NOT NULL AND question_hash IS NOT NULL WINDOW w AS (PARTITION BY setup,question_hash)
), sources AS (
    SELECT *,row_number() OVER(PARTITION BY setup,source ORDER BY started DESC,trace_id,call_id,report) AS exemplar
    FROM comparable WHERE witness=1 AND named=total AND lo=hi AND lo='confirmed' AND first_source=last_source
), examples AS (
    SELECT s.*,row_number() OVER(PARTITION BY setup ORDER BY source) AS side,
        json_object('question',c.preview->>'$.user.text','source',s.source,'thread',s.thread,'original',s.choices->>'original','edited',s.choices->>'edited',
            'trace',s.trace_id,'call',s.call_id,'report',s.report) AS evidence
    FROM sources s JOIN conversation_calls c USING(trace_id,call_id) WHERE exemplar=1
) SELECT json_object('setup',setup,'examples',json_array(
    json(max(evidence) FILTER(WHERE side=1)),json(max(evidence) FILTER(WHERE side=2))))
    FROM examples GROUP BY setup HAVING count(*)>=2 ORDER BY max(started) DESC,setup LIMIT ? OFFSET ?";
