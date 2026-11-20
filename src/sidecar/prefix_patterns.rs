//! Index the prediction and its checked outcome without copying scores or trial text.
use super::findings::digest;
use nexusweave::{
    replay::{candidates, shadow},
    Call, Run,
};
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS prefix_checks (
    trace_id TEXT NOT NULL, call_id TEXT NOT NULL, report TEXT NOT NULL, design TEXT NOT NULL,
    prefix TEXT NOT NULL, source TEXT, verdict TEXT NOT NULL, input TEXT NOT NULL, setup TEXT, question_hash TEXT, choices TEXT NOT NULL,
    PRIMARY KEY(trace_id,call_id,report),
    FOREIGN KEY(trace_id,call_id) REFERENCES captured_calls ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS prefix_trials (
    trace_id TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE, call_id TEXT NOT NULL, runner TEXT NOT NULL,
    report TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(trace_id,call_id,runner,report)
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
    let uploads = uploads(tx, &run.id, &call.id)?;
    let evidence = upload_origin(call);
    let trials = trials(&run.id, call, &uploads, &evidence);
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
            let trial_key = selection.map(|s| {
                digest(
                    json!([
                        raw.captured_request_sha256,
                        s["source"],
                        s["version"],
                        s["original_sha256"],
                        s["edit"],
                        preferred(&r["conditions"][0]),
                        preferred(s)
                    ])
                    .to_string(),
                )
            });
            let choices = json!({"reference_set":r["reference"]["set"],"reference_outcome":r["reference"]["outcome"],"reference_prediction":r["reference"]["prediction"],"trial_key":trial_key,"alternatives":r["alternatives"],"model":r["model"],"revision":r["revision"],"runtime":r["runtime"],"scorer":r["scorer"],
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

pub(super) fn uploads(
    db: &Connection,
    trace: &str,
    call: &str,
) -> rusqlite::Result<Vec<shadow::Report>> {
    let mut stmt = db.prepare_cached(
        "SELECT body FROM prefix_trials WHERE trace_id=? AND call_id=? ORDER BY runner,report",
    )?;
    let results = stmt
        .query_map(params![trace, call], |r| {
            let body: String = r.get(0)?;
            serde_json::from_str(&body).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })?
        .collect();
    results
}

pub(super) fn upload_origin(call: &Call) -> nexusweave::Evidence {
    nexusweave::Evidence {
        field: "sidecar.prefix_trials".into(),
        span_id: call.span_id.clone(),
        message_index: None,
        part_index: None,
    }
}

pub(super) fn captured_report<'a>(call: &'a Call, id: &str) -> Option<&'a shadow::Report> {
    let mut reports = call
        .shadow_probes
        .as_ref()?
        .reports
        .as_ref()?
        .iter()
        .filter(|r| r.id == id);
    let first = reports.next()?;
    reports.next().is_none().then_some(first)
}

pub(super) fn trials<'a>(
    id: &'a str,
    call: &'a Call,
    uploads: &'a [shadow::Report],
    evidence: &'a nexusweave::Evidence,
) -> Vec<shadow::RecordedProbe<'a>> {
    let mut trials = shadow::recorded(id, call);
    trials.extend(
        uploads
            .iter()
            .filter(|r| captured_report(call, &r.id) != Some(*r))
            .filter_map(|r| shadow::attached(id, call, r, evidence).ok()),
    );
    trials
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

pub(super) const SCOPED: &str = ", scoped_checks AS (
    SELECT p.*,t.thread_key,t.thread,c.started FROM prefix_checks p JOIN selected t ON t.id=p.trace_id
    JOIN captured_calls c USING(trace_id,call_id) WHERE t.thread_key IS NOT NULL";

// Scope before looking for an existing test, then filter to individual witnesses.
pub(super) const TRIAL_COVERAGE: &str = ", coverage AS (
    SELECT *,verdict='untested' AND choices->>'trial_key' IS NOT NULL AND
        max(verdict IN ('confirmed','disagreed','unresolved','mixed')) OVER
        (PARTITION BY design,choices->>'trial_key')=0 AS needs_trial FROM scoped_checks
)";

pub(super) const GROUPS: &str = " SELECT json_object('design',design,'prefix',prefix,
    'model',choices->>'model','revision',choices->>'revision','runtime',choices->>'runtime','scorer',choices->>'scorer','alternatives',choices->'alternatives',
    'conversations',count(DISTINCT thread_key),'inputs',count(DISTINCT input),
    'exceptions',count(DISTINCT thread_key) FILTER (WHERE verdict IN ('disagreed','unselected_change','mixed')),
    'confirmed',count(DISTINCT thread_key) FILTER (WHERE verdict='confirmed'),
    'referenced',count(DISTINCT thread_key) FILTER (WHERE choices->>'reference_set' IS NOT NULL),
    'restored',count(DISTINCT thread_key) FILTER (WHERE choices->>'reference_outcome'='restored'),
    'lost',count(DISTINCT thread_key) FILTER (WHERE choices->>'reference_outcome'='lost'),
    'forecast_restore',count(DISTINCT thread_key) FILTER (WHERE needs_trial AND choices->>'reference_prediction'='restored'),
    'untested',count(DISTINCT thread_key) FILTER (WHERE verdict='untested'),
    'needs_trial',count(DISTINCT thread_key) FILTER (WHERE needs_trial),
    'experiments',count(DISTINCT choices->>'trial_key') FILTER (WHERE needs_trial))
    FROM checks GROUP BY design,prefix ORDER BY
    count(DISTINCT thread_key) FILTER (WHERE choices->>'reference_outcome'='restored') DESC,
    count(DISTINCT thread_key) FILTER (WHERE choices->>'reference_outcome'='lost') DESC,
    count(DISTINCT thread_key) FILTER (WHERE verdict IN ('disagreed','unselected_change','mixed')) DESC,
    count(DISTINCT thread_key) FILTER (WHERE needs_trial) DESC,
    count(DISTINCT thread_key) FILTER (WHERE verdict='confirmed') DESC,
    count(DISTINCT thread_key) DESC,max(started) DESC,design LIMIT ? OFFSET ?";

pub(super) const BREAKDOWN: &str = " SELECT json_object('source',source,'verdict',verdict,
    'conversations',count(DISTINCT thread_key),'inputs',count(DISTINCT input))
    FROM checks GROUP BY source,verdict ORDER BY count(DISTINCT thread_key) DESC,source,verdict LIMIT ? OFFSET ?";

pub(super) const REFERENCES: &str = " SELECT json_object('source',source,'reference_set',choices->>'reference_set',
    'outcome',choices->>'reference_outcome','conversations',count(DISTINCT thread_key),'inputs',count(DISTINCT input))
    FROM checks WHERE choices->>'reference_set' IS NOT NULL GROUP BY source,choices->>'reference_set',choices->>'reference_outcome'
    ORDER BY count(DISTINCT thread_key) DESC,source,choices->>'reference_set',choices->>'reference_outcome' LIMIT ? OFFSET ?";

pub(super) const CASES: &str = ", witnesses AS (
    SELECT *,row_number() OVER(PARTITION BY thread_key ORDER BY started DESC,trace_id,call_id,report) AS sample
    FROM checks) SELECT json_object('thread',w.thread,'thread_key',w.thread_key,'trace',w.trace_id,'call',w.call_id,
    'report',w.report,'source',w.source,'verdict',w.verdict,'trial_key',w.choices->>'trial_key','question',c.preview->>'$.user.text')
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
