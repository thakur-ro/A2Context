use super::{findings, recommendations};
use crate::{report_store::Report, Result};
use nexusweave::{activation, otlp, replay, Run};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const MAX_TRACE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPORT_BYTES_PER_TRACE: usize = 4 * 1024 * 1024;
const PAGE: usize = 15;

pub struct Store(Connection);
type Rejected = (u16, String);

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS policy (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS traces (
                 id TEXT PRIMARY KEY, seen INTEGER NOT NULL, start INTEGER, org TEXT, user TEXT, thread TEXT,
                 thread_key TEXT, workflow TEXT, source TEXT, metadata TEXT NOT NULL,
                 raw TEXT, run TEXT, calls INTEGER NOT NULL, with_inputs INTEGER NOT NULL, with_request INTEGER NOT NULL, with_rules INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS traces_seen ON traces(seen);
             CREATE INDEX IF NOT EXISTS traces_owner ON traces(org,user,workflow,start);
             CREATE INDEX IF NOT EXISTS traces_thread ON traces(thread_key,start);
             CREATE TABLE IF NOT EXISTS metadata (
                 trace_id TEXT REFERENCES traces(id) ON DELETE CASCADE, key TEXT, value TEXT, PRIMARY KEY(trace_id,key)
             );
             CREATE INDEX IF NOT EXISTS metadata_lookup ON metadata(key,value,trace_id);
             CREATE TABLE IF NOT EXISTS observations (
                 trace_id TEXT REFERENCES traces(id) ON DELETE CASCADE, n INTEGER, call_id TEXT, kind TEXT,
                 group_id TEXT, observed INTEGER, detail TEXT, PRIMARY KEY(trace_id,n)
             );
             CREATE INDEX IF NOT EXISTS observations_type ON observations(kind,group_id,observed);
             CREATE INDEX IF NOT EXISTS observations_factor ON observations(json_extract(detail,'$.factor'),observed);
             CREATE INDEX IF NOT EXISTS observations_prior ON observations(json_extract(detail,'$.prior.partition'),
                observed DESC,trace_id,n) WHERE json_extract(detail,'$.prior.partition') IS NOT NULL;
             CREATE TABLE IF NOT EXISTS reports (
                 trace_id TEXT REFERENCES traces(id) ON DELETE CASCADE, kind TEXT, call_id TEXT, decision_id TEXT, id TEXT, body TEXT,
                 PRIMARY KEY(trace_id,kind,call_id,decision_id,id)
             );",
        )?;
        Ok(Self(db))
    }

    pub fn bind_policy(&self, capture: bool, keys: &BTreeSet<String>) -> Result<()> {
        let policy = serde_json::to_string(&(capture, keys))?;
        self.0
            .execute("INSERT OR IGNORE INTO policy VALUES (1,?1)", [&policy])?;
        let previous: String =
            self.0
                .query_row("SELECT value FROM policy WHERE id=1", [], |row| row.get(0))?;
        if previous != policy {
            return Err(
                "capture settings differ from the existing database. Use a new database.".into(),
            );
        }
        Ok(())
    }

    pub fn purge(&self, cutoff: i64) -> Result<usize> {
        Ok(self
            .0
            .execute("DELETE FROM traces WHERE seen < ?1", [cutoff])?)
    }

    pub fn ingest(
        &mut self,
        doc: &Value,
        policy: &otlp::CapturePolicy<'_>,
        now: i64,
    ) -> std::result::Result<(), Rejected> {
        let tx = self.0.transaction().map_err(storage)?;
        let mut load_failed = false;
        let mut loaded_bytes = 0;
        let batch = otlp::prepare_stored(doc, policy, |id| {
            let read = || -> Result<_> {
                let saved: Option<String> = tx
                    .query_row("SELECT raw FROM traces WHERE id=?1", [id], |row| row.get(0))
                    .optional()?
                    .flatten();
                Ok(saved)
            };
            let saved = read().map_err(|error| {
                load_failed = true;
                error.to_string()
            })?;
            loaded_bytes += saved.as_ref().map_or(0, String::len);
            if loaded_bytes > MAX_LOAD_BYTES {
                return Err(
                    "existing trace snapshots exceed 16 MiB per export. Send smaller batches."
                        .into(),
                );
            }
            saved
                .map(|raw| serde_json::from_str(&raw))
                .transpose()
                .map(Option::unwrap_or_default)
                .map_err(|error| {
                    load_failed = true;
                    error.to_string()
                })
        })
        .map_err(|error| {
            if load_failed {
                storage(error)
            } else {
                (400, error)
            }
        })?;
        for (id, raw, run) in batch.into_updates() {
            let metadata = raw.metadata();
            let encoded = serde_json::to_string(&raw).map_err(storage)?;
            if encoded.len() > MAX_TRACE_BYTES {
                return Err((413, "merged sidecar trace exceeds 2 MiB".into()));
            }
            save_trace(
                &tx,
                &id,
                run.as_ref(),
                Some((&raw, encoded)),
                &metadata,
                now,
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }

    pub fn seed(
        &mut self,
        runs: &[Run],
        reports: &[replay::Replay],
        activation: &activation::Report,
        now: i64,
    ) -> Result<()> {
        let tx = self.0.transaction()?;
        for run in runs {
            nexusweave::validate(run)?;
            save_trace(
                &tx,
                &run.id,
                Some(run),
                None,
                &BTreeMap::from([("service.name".into(), "returns-demo".into())]),
                now,
            )?;
        }
        for report in reports {
            replay::validate_report(report, runs)?;
            save_report(&tx, report)?;
        }
        activation.validate(runs)?;
        save_report(&tx, activation)?;
        for run in runs {
            refresh_findings(&tx, run)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn attach<T: Report + Serialize>(
        &mut self,
        report: &T,
        now: i64,
    ) -> std::result::Result<bool, Rejected> {
        let tx = self.0.transaction().map_err(storage)?;
        let location = report.location();
        let run = load_run(&tx, &location.run_id).map_err(storage)?.ok_or((
            409,
            "record the target trace before attaching a report".into(),
        ))?;
        let call = run
            .calls
            .iter()
            .find(|call| call.id == location.call_id)
            .ok_or((400, "report call is unavailable".into()))?;
        report
            .validate_for(&run, call)
            .map_err(|error| (400, error))?;
        let body: String = serde_json::to_string(report).map_err(storage)?;
        let prior: Option<String> = tx.query_row(
            "SELECT body FROM reports WHERE trace_id=?1 AND kind=?2 AND call_id=?3 AND decision_id=?4 AND id=?5",
            params![run.id,T::DIRECTORY,location.call_id,location.decision_id,report.id()], |row| row.get(0)
        ).optional().map_err(storage)?;
        if let Some(prior) = prior {
            return if &serde_json::from_str::<T>(&prior).map_err(storage)? == report {
                Ok(false)
            } else {
                Err((409, "report ID already has different evidence".into()))
            };
        }
        let bytes: usize = tx
            .query_row(
                "SELECT coalesce(sum(length(cast(body AS blob))),0) FROM reports WHERE trace_id=?1",
                [&run.id],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if bytes + body.len() > MAX_REPORT_BYTES_PER_TRACE {
            return Err((413, "reports for this trace exceed 4 MiB".into()));
        }
        save_report(&tx, report).map_err(storage)?;
        tx.execute(
            "UPDATE traces SET seen=?2 WHERE id=?1",
            params![run.id, now],
        )
        .map_err(storage)?;
        refresh_findings(&tx, &run).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(true)
    }

    pub fn upgrade_observations(&mut self) -> Result<()> {
        let version: i64 = self
            .0
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 7 {
            return Ok(());
        }
        if !(0..7).contains(&version) {
            return Err("unsupported sidecar database version".into());
        }
        let mut after = String::new();
        loop {
            let ids = {
                let mut stmt = self.0.prepare(
                    "SELECT id FROM traces WHERE id>?1 AND run IS NOT NULL ORDER BY id LIMIT 16",
                )?;
                let rows = stmt.query_map([&after], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let Some(last) = ids.last() else { break };
            let tx = self.0.transaction()?;
            for id in &ids {
                if let Some(run) = load_run(&tx, id)? {
                    refresh_findings(&tx, &run)?;
                }
            }
            tx.commit()?;
            after.clone_from(last);
        }
        self.0.pragma_update(None, "user_version", 7)?;
        Ok(())
    }

    pub fn trace(&self, id: &str) -> Result<Option<Run>> {
        load_run(&self.0, id)
    }

    pub fn replays(&self, id: &str) -> Result<Vec<replay::Replay>> {
        load_reports(&self.0, id)
    }

    pub fn activations(&self, id: &str) -> Result<Vec<activation::Report>> {
        load_reports(&self.0, id)
    }

    pub fn legacy_runs(&self, thread: Option<&str>) -> Result<Vec<Run>> {
        let query = if thread.is_some() {
            "SELECT run,length(cast(run AS blob)) FROM traces WHERE thread_key=?1 AND run IS NOT NULL ORDER BY start DESC,id LIMIT 9"
        } else {
            "SELECT run,length(cast(run AS blob)) FROM traces WHERE run IS NOT NULL ORDER BY start DESC,id LIMIT 9"
        };
        let mut stmt = self.0.prepare(query)?;
        let mut rows = stmt.query(params_from_iter(thread))?;
        let mut runs = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            bytes += row.get::<_, usize>(1)?;
            if bytes > 8 * 1024 * 1024 || runs.len() == 8 {
                return Err(
                    "this scope is too large for a legacy view. Open it in Findings, Traces, or Threads.".into(),
                );
            }
            runs.push(serde_json::from_str(&row.get::<_, String>(0)?)?);
        }
        Ok(runs)
    }

    pub fn coverage(&self, filters: &BTreeMap<String, String>) -> Result<Value> {
        let mut sql = "SELECT count(*),coalesce(sum(calls),0),coalesce(sum(with_inputs),0),
            coalesce(sum(with_request),0),coalesce(sum(with_rules),0) FROM traces t WHERE t.run IS NOT NULL".to_owned();
        let mut args = Vec::new();
        append_scope(&mut sql, &mut args, filters)?;
        append_window(&mut sql, &mut args, "t.start", filters)?;
        Ok(self.0.query_row(&sql, params_from_iter(args), |row| {
            Ok(
                json!({"traces": row.get::<_,i64>(0)?, "inference_spans": row.get::<_,i64>(1)?,
                "with_inputs": row.get::<_,i64>(2)?, "with_request": row.get::<_,i64>(3)?,
                "with_rules": row.get::<_,i64>(4)?}),
            )
        })?)
    }

    pub fn prior_edits(&self, partition: &str, current: &str) -> Result<Value> {
        let sql = "WITH recent AS MATERIALIZED (
            SELECT detail, trace_id, call_id, observed, n FROM observations
            WHERE json_extract(detail,'$.prior.partition')=?1
            ORDER BY observed DESC,trace_id,n LIMIT 513
        ), tested AS (
            SELECT * FROM recent ORDER BY observed DESC,trace_id,n LIMIT 512
        ), requests AS (
            SELECT json_extract(detail,'$.prior.setup') AS setup, json_extract(detail,'$.prior.body') AS body,
                min(json_extract(detail,'$.effect_pp')<0) AS lo,max(json_extract(detail,'$.effect_pp')<0) AS hi,
                min(json_object('trace',trace_id,'call',call_id,'source',json_extract(detail,'$.sources[0].source'),
                    'version',json_extract(detail,'$.sources[0].version'),'replacement',json_extract(detail,'$.prior.replacement'),
                    'runner',json_extract(detail,'$.runner'),'rule',json_extract(detail,'$.match_rule'),
                    'outcome',json_extract(detail,'$.outcome'),'report',json_extract(detail,'$.measurement.replay_id'),
                    'decision',json_extract(detail,'$.measurement.location.decision_id'),
                    'context',json_extract(detail,'$.measurement.context_id'))) AS example
            FROM tested WHERE json_extract(detail,'$.prior.body')!=?2 GROUP BY setup,body
        ), ranked AS (
            SELECT setup,count(*) AS bodies,sum(lo) AS lower,sum(1-hi) AS equal,sum(hi-lo) AS mixed,min(example) AS example
            FROM requests GROUP BY setup HAVING count(*)>=2
            ORDER BY lower DESC,mixed ASC,bodies DESC,setup LIMIT 5
        ) SELECT json_object('status','checked','truncated',(SELECT count(*)>512 FROM recent),
            'edits',json_group_array(json_object('requests',bodies,'lower',lower,'equal',equal,'mixed',mixed,
            'example',json(example)))) FROM ranked";
        let data: String = self
            .0
            .query_row(sql, rusqlite::params![partition, current], |row| row.get(0))?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn list(&self, view: &str, filters: &BTreeMap<String, String>) -> Result<Value> {
        let mut sql = "WITH selected AS (SELECT t.id,t.start,t.org,t.user,t.thread,t.thread_key,t.workflow,t.source,t.calls,t.with_inputs,t.metadata FROM traces t WHERE t.run IS NOT NULL".to_owned();
        let mut args = Vec::new();
        append_scope(&mut sql, &mut args, filters)?;
        if matches!(view, "traces" | "threads") {
            append_window(&mut sql, &mut args, "t.start", filters)?;
        }
        sql.push(')');
        let offset = filters
            .get("offset")
            .map(|offset| offset.parse::<u32>())
            .transpose()?
            .unwrap_or(0);
        if matches!(
            view,
            "findings" | "factors" | "impact" | "occurrences" | "order-tests"
        ) {
            sql.push_str(", events AS (SELECT o.*,t.thread_key,t.thread,t.org,json_extract(o.detail,'$.factor') AS factor,
                CASE WHEN o.kind IN ('candidate','token','shadow') OR
                    (o.kind='replay' AND json_extract(o.detail,'$.effect_pp') != 0) THEN 1 ELSE 0 END AS changed
                FROM observations o JOIN selected t ON t.id=o.trace_id WHERE 1");
            for (key, field) in [
                ("kind", "o.kind"),
                ("group", "o.group_id"),
                ("factor", "json_extract(o.detail,'$.factor')"),
            ] {
                if let Some(value) = filters.get(key).filter(|value| !value.is_empty()) {
                    sql.push_str(&format!(" AND {field}=?"));
                    args.push(value.clone());
                }
            }
            append_window(&mut sql, &mut args, "o.observed", filters)?;
            sql.push(')');
        }
        if matches!(view, "order-tests" | "factors") {
            sql.push_str(super::order::MATCHED);
        }
        match view {
            "order-tests" => sql.push_str(" SELECT json_object('factor',factor,'design',design,
                'differs',differs,'total',count(*) OVER (),'differing',sum(differs) OVER (),
                'forward',json(forward),'reverse',json(reverse)) FROM pairs
                ORDER BY differs DESC,factor,design LIMIT ? OFFSET ?"),
            "impact" => {
                sql.push_str(", replay_by_thread AS (
                    SELECT thread_key, max(kind='replay' AND changed=1) AS shifted,
                        max(kind='replay' AND json_extract(detail,'$.effect_pp')=0) AS held
                    FROM events WHERE thread_key IS NOT NULL GROUP BY thread_key
                ) SELECT json_object('conversations',count(DISTINCT thread_key),
                    'changed',count(DISTINCT CASE WHEN changed=1 THEN thread_key END),
                    'replay_outcomes',(SELECT json_object(
                        'changed_only',count(*) FILTER (WHERE shifted AND NOT held),
                        'equal_only',count(*) FILTER (WHERE held AND NOT shifted),
                        'mixed',count(*) FILTER (WHERE shifted AND held),
                        'unmeasured',count(*) FILTER (WHERE NOT shifted AND NOT held))
                        FROM replay_by_thread),
                    'unlinked_requests',count(DISTINCT CASE WHEN thread_key IS NULL THEN trace_id END),
                    'factors',count(DISTINCT factor)) FROM events");
                let row: String = self.0.query_row(&sql, params_from_iter(args), |row| row.get(0))?;
                return Ok(serde_json::from_str(&row)?);
            }
            "factors" => sql.push_str(", grouped AS (
                SELECT factor,org,json_extract(min(detail),'$.sources') AS sources,
                    count(DISTINCT thread_key) AS conversations,count(DISTINCT trace_id) AS requests,
                    count(DISTINCT CASE WHEN changed=1 THEN trace_id END) AS changed_requests,
                    count(DISTINCT group_id) + (factor IN (SELECT factor FROM pairs)) AS recommendations,max(observed) AS latest,
                    CASE WHEN factor IN (SELECT factor FROM pairs WHERE differs) THEN -3 ELSE
                    min(CASE WHEN kind='replacement' THEN -2 WHEN json_extract(detail,'$.joint_only')=1 THEN -1 WHEN kind='candidate' AND json_extract(detail,'$.baseline')*json_extract(detail,'$.edited')<0 THEN 0
                        WHEN kind='replay' AND changed=1 THEN 1 WHEN kind='shadow' THEN 2 WHEN kind='token' THEN 3
                        WHEN kind='conflict' THEN 4 WHEN kind='candidate' THEN 5 WHEN kind='duplicate' AND json_extract(detail,'$.dialogue_only')=1 THEN 8 WHEN kind='duplicate' THEN 6 ELSE 7 END) END AS signal
                FROM events GROUP BY factor)
                SELECT json_object('id',factor,'sources',json(sources),'org',org,'conversations',conversations,
                    'requests',requests,'changed',changed_requests,'recommendations',recommendations,
                    'signal',CASE signal WHEN -3 THEN 'order' WHEN -2 THEN 'replacement' WHEN -1 THEN 'joint' WHEN 0 THEN 'flip' WHEN 1 THEN 'replay' WHEN 2 THEN 'shadow' WHEN 3 THEN 'token'
                        WHEN 4 THEN 'conflict' WHEN 5 THEN 'score' WHEN 6 THEN 'duplicate' WHEN 8 THEN 'dialogue' ELSE 'equal' END)
                FROM grouped ORDER BY signal,conversations DESC,requests DESC,latest DESC,factor LIMIT ? OFFSET ?"),
            "occurrences" => sql.push_str(" SELECT json_object('trace',trace_id,'call',call_id,'thread',thread,
                'thread_key',thread_key,'observed',observed,'kind',kind,'detail',json(detail)) FROM events
                ORDER BY observed DESC,trace_id,call_id,n LIMIT ? OFFSET ?"),
            "findings" => {
                sql.push_str(", groups AS (
                    SELECT group_id,kind,count(*) AS hits,count(DISTINCT trace_id) AS traces,
                           count(DISTINCT thread_key) AS threads,max(observed) AS latest,
                           CASE
                             WHEN kind='replacement' THEN -4
                             WHEN kind='candidate' AND min(json_extract(detail,'$.baseline')*json_extract(detail,'$.edited'))<0 THEN -3
                             WHEN kind IN ('shadow','token') THEN -2
                             WHEN kind='candidate' THEN -1
                             WHEN kind='conflict' AND count(DISTINCT trace_id)>1 THEN 0
                             WHEN kind='replay' AND max(abs(json_extract(detail,'$.effect_pp')))>0 THEN 1
                             WHEN kind='conflict' THEN 2
                             WHEN kind='duplicate' AND min(coalesce(json_extract(detail,'$.dialogue_only'),0))=1 THEN 5
                             WHEN kind='duplicate' THEN 3
                             ELSE 4 END AS priority
                    FROM events GROUP BY group_id,kind
                    ORDER BY priority,threads DESC,traces DESC,latest DESC,group_id LIMIT ? OFFSET ?
                 )
                 SELECT json_object('id',g.group_id,'kind',g.kind,'occurrences',g.hits,'traces',g.traces,
                     'threads',g.threads,'latest',g.latest,'priority',g.priority,
                     'evidence',json((SELECT json_object('thread_key',e.thread_key,'thread',e.thread,'trace_id',e.trace_id,'call_id',e.call_id,'observed',e.observed,'detail',json(e.detail))
                       FROM events e WHERE e.group_id=g.group_id
                       ORDER BY CASE
                                  WHEN json_extract(e.detail,'$.joint_only')=1 THEN -1
                                  WHEN e.kind='candidate' AND json_extract(e.detail,'$.baseline')*json_extract(e.detail,'$.edited')<0 THEN 0
                                  WHEN e.kind='replay' AND abs(json_extract(e.detail,'$.effect_pp'))>0 THEN 0
                                  ELSE 1 END,
                                e.observed DESC,e.trace_id,e.call_id,e.n LIMIT 1)),
                     'trend',json((SELECT json_group_array(json_object('day',day,'count',hits)) FROM
                       (SELECT date(observed,'unixepoch') AS day,count(*) AS hits FROM events e
                        WHERE e.group_id=g.group_id AND observed IS NOT NULL GROUP BY day ORDER BY day DESC LIMIT 14))))
                 FROM groups g ORDER BY priority,threads DESC,traces DESC,latest DESC,group_id");
            }
            "traces" => {
                sql.push_str(" SELECT json_object('id',id,'start',start,'org',org,'user',user,'thread',thread,
                      'thread_key',thread_key,'workflow',workflow,'source',source,'calls',calls,'with_inputs',with_inputs,
                      'metadata',json(metadata)) FROM selected");
                sql.push_str(if filters.contains_key("thread") {
                    " ORDER BY start IS NULL,start,id LIMIT ? OFFSET ?"
                } else {
                    " ORDER BY start DESC,id LIMIT ? OFFSET ?"
                });
            }
            "threads" => sql.push_str(" SELECT json_object('id',thread_key,'thread',thread,'org',org,'user',user,
                      'start',max(start),'trace_count',count(*)) FROM selected WHERE thread_key IS NOT NULL
                      GROUP BY thread_key ORDER BY max(start) DESC,thread_key LIMIT ? OFFSET ?"),
            _ => return Err("unknown view".into()),
        }
        args.extend([(PAGE + 1).to_string(), offset.to_string()]);
        let mut stmt = self.0.prepare(&sql)?;
        let mut rows: Vec<Value> = stmt
            .query_map(params_from_iter(args), |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<_>>()?;
        let next = (rows.len() > PAGE).then(|| offset.saturating_add(PAGE as u32));
        rows.truncate(PAGE);
        if view == "findings" {
            for row in &mut rows {
                let kind = row["kind"].as_str().ok_or("finding kind missing")?;
                row["action"] = serde_json::to_value(recommendations::action(
                    kind,
                    &row["evidence"]["detail"],
                ))?;
            }
        }
        Ok(json!({"rows":rows,"next_offset":next}))
    }
}

fn append_scope(
    sql: &mut String,
    args: &mut Vec<String>,
    filters: &BTreeMap<String, String>,
) -> Result<()> {
    for (key, column) in [
        ("org", "org"),
        ("user", "user"),
        ("workflow", "workflow"),
        ("thread", "thread_key"),
    ] {
        if let Some(value) = filters.get(key).filter(|value| !value.is_empty()) {
            sql.push_str(&format!(" AND t.{column}=?"));
            args.push(value.clone());
        }
    }
    if let Some(key) = filters.get("meta_key").filter(|key| !key.is_empty()) {
        let value = filters
            .get("meta_value")
            .ok_or("metadata filter needs a value")?;
        sql.push_str(" AND EXISTS (SELECT 1 FROM metadata m WHERE m.key=? AND m.value=? AND m.trace_id=t.id)");
        args.extend([key.clone(), value.clone()]);
    }
    Ok(())
}

fn append_window(
    sql: &mut String,
    args: &mut Vec<String>,
    column: &str,
    filters: &BTreeMap<String, String>,
) -> Result<()> {
    for (key, operator) in [("after", ">="), ("before", "<")] {
        if let Some(day) = filters.get(key).filter(|day| !day.is_empty()) {
            let day = OffsetDateTime::parse(&format!("{day}T00:00:00Z"), &Rfc3339)?;
            sql.push_str(&format!(" AND {column}{operator}CAST(? AS INTEGER)"));
            args.push(day.unix_timestamp().to_string());
        }
    }
    Ok(())
}

fn at(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .map(OffsetDateTime::unix_timestamp)
}

fn save_trace(
    tx: &Transaction<'_>,
    id: &str,
    run: Option<&Run>,
    raw: Option<(&otlp::StoredTrace, String)>,
    metadata: &BTreeMap<String, String>,
    seen: i64,
) -> Result<()> {
    let start = run.and_then(|run| {
        run.calls
            .iter()
            .map(|call| call.started_at.as_deref())
            .chain(run.spans.iter().map(|span| span.started_at.as_deref()))
            .filter_map(at)
            .min()
    });
    let key = run
        .and_then(|run| {
            run.conversation_id.as_deref().map(|thread| {
                serde_json::to_string(&(&run.source, &run.organization_id, &run.user_id, thread))
            })
        })
        .transpose()?;
    let inference = run.into_iter().flat_map(|run| &run.calls).filter(|call| {
        raw.as_ref().map_or_else(
            || {
                call.retrieval.is_none()
                    && call.tool_results.is_empty()
                    && !call
                        .decisions
                        .iter()
                        .any(|decision| decision.kind == "tool_execution")
            },
            |(record, _)| {
                call.span_id
                    .as_deref()
                    .is_some_and(|id| record.inference_span(id))
            },
        )
    });
    let calls = inference.clone().count();
    let with_inputs = inference
        .clone()
        .filter(|call| {
            call.context
                .iter()
                .any(|item| item.availability == nexusweave::Availability::Included)
        })
        .count();
    let with_rules = inference
        .clone()
        .filter(|call| {
            call.context.iter().any(|item| {
                item.availability == nexusweave::Availability::Included
                    && item.rule.as_ref().is_some_and(|rule| rule.value.is_some())
            })
        })
        .count();
    let with_request = inference
        .filter(|call| call.recorded_request.is_some())
        .count();
    tx.execute("INSERT INTO traces VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
        ON CONFLICT(id) DO UPDATE SET seen=excluded.seen,start=excluded.start,org=excluded.org,user=excluded.user,
        thread=excluded.thread,thread_key=excluded.thread_key,workflow=excluded.workflow,source=excluded.source,
        metadata=excluded.metadata,raw=excluded.raw,run=excluded.run,calls=excluded.calls,with_inputs=excluded.with_inputs,
        with_request=excluded.with_request,with_rules=excluded.with_rules",
        params![id, seen, start, run.and_then(|run| run.organization_id.as_deref()), run.and_then(|run| run.user_id.as_deref()),
                run.and_then(|run| run.conversation_id.as_deref()), key, metadata.get("service.name"), run.map(|run| &run.source),
                serde_json::to_string(metadata)?, raw.map(|(_, stored)| stored), run.map(serde_json::to_string).transpose()?, calls, with_inputs, with_request, with_rules])?;
    tx.execute("DELETE FROM metadata WHERE trace_id=?1", [id])?;
    for (key, value) in metadata {
        tx.execute(
            "INSERT INTO metadata VALUES (?1,?2,?3)",
            params![id, key, value],
        )?;
    }
    if let Some(run) = run {
        refresh_findings(tx, run)?;
    }
    Ok(())
}

fn save_report<T: Report + Serialize>(tx: &Transaction<'_>, report: &T) -> Result<()> {
    let location = report.location();
    tx.execute(
        "INSERT INTO reports VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            location.run_id,
            T::DIRECTORY,
            location.call_id,
            location.decision_id,
            report.id(),
            serde_json::to_string(report)?
        ],
    )?;
    Ok(())
}

fn load_run(db: &Connection, id: &str) -> Result<Option<Run>> {
    let run: Option<String> = db
        .query_row("SELECT run FROM traces WHERE id=?1", [id], |row| row.get(0))
        .optional()?
        .flatten();
    Ok(run.map(|run| serde_json::from_str(&run)).transpose()?)
}

fn load_reports<T: Report>(db: &Connection, id: &str) -> Result<Vec<T>> {
    let mut stmt = db.prepare(
        "SELECT body FROM reports WHERE trace_id=?1 AND kind=?2 ORDER BY call_id,decision_id,id",
    )?;
    let rows = stmt.query_map([id, T::DIRECTORY], |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

fn refresh_findings(tx: &Transaction<'_>, run: &Run) -> Result<()> {
    tx.execute("DELETE FROM observations WHERE trace_id=?1", [&run.id])?;
    let reports = load_reports(tx, &run.id)?;
    let activations = load_reports(tx, &run.id)?;
    let observations = findings::derive(run, &reports, &activations)?;
    let mut insert = tx.prepare("INSERT INTO observations VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
    for (index, mut observation) in observations.into_iter().enumerate() {
        observation.detail["factor"] = findings::factor_id(run, &observation).into();
        insert.execute(params![
            run.id,
            index,
            observation.call_id,
            observation.kind,
            observation.group,
            at(observation.observed.as_deref()),
            serde_json::to_string(&observation.detail)?
        ])?;
    }
    Ok(())
}

fn storage(error: impl std::fmt::Display) -> Rejected {
    eprintln!("Sidecar storage error: {error}");
    (503, "sidecar storage unavailable".into())
}
