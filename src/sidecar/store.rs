mod prefix;

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
    pub(super) fn workflows(&self) -> Result<Vec<String>> {
        let mut stmt = self.0.prepare(
            "SELECT DISTINCT workflow FROM traces WHERE run IS NOT NULL AND workflow > '' ORDER BY workflow",
        )?;
        let workflows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(workflows)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        super::position_plan::register_pair_id(&db)?;
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
             CREATE INDEX IF NOT EXISTS traces_coverage ON traces(workflow,org,user,thread_key,start,id,calls,with_inputs,with_request,with_rules)
                 WHERE run IS NOT NULL;
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
             CREATE INDEX IF NOT EXISTS observations_prior_dispatch ON observations(json_extract(detail,'$.dispatch_history.partition'),
                observed DESC,trace_id,n) WHERE json_extract(detail,'$.dispatch_history.partition') IS NOT NULL;
             CREATE TABLE IF NOT EXISTS position_results (
                 pair TEXT NOT NULL, id TEXT NOT NULL, first_trace TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE,
                 second_trace TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE, body TEXT NOT NULL, summary TEXT NOT NULL,
                 PRIMARY KEY(pair,id)
             );
             CREATE INDEX IF NOT EXISTS position_results_first ON position_results(first_trace);
             CREATE INDEX IF NOT EXISTS position_results_second ON position_results(second_trace);
             CREATE TABLE IF NOT EXISTS reports (
                 trace_id TEXT REFERENCES traces(id) ON DELETE CASCADE, kind TEXT, call_id TEXT, decision_id TEXT, id TEXT, body TEXT,
                 PRIMARY KEY(trace_id,kind,call_id,decision_id,id)
             );",
        )?;
        db.execute_batch(super::capture::SCHEMA)?;
        db.execute_batch(super::conversation::SCHEMA)?;
        db.execute_batch(super::activity_search::SCHEMA)?;
        db.execute_batch(super::target_patterns::SCHEMA)?;
        db.execute_batch(super::prefix_patterns::SCHEMA)?;
        db.execute_batch(super::screened::INDEX)?;
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
        if !capture {
            self.0.execute(
                "UPDATE activity_index_progress SET cursor=NULL WHERE id=1 AND cursor IS NOT NULL",
                [],
            )?;
        }
        Ok(())
    }

    pub fn purge(&self, cutoff: i64) -> Result<usize> {
        Ok(self
            .0
            .execute("DELETE FROM traces WHERE seen < ?1", [cutoff])?)
    }

    pub(super) fn backfill_activity_search(&mut self) -> Result<bool> {
        const BATCH: usize = 24;
        let tx = self.0.transaction()?;
        let Some(cursor) = super::activity_search::cursor(&tx)? else {
            return Ok(false);
        };
        let traces = tx
            .prepare_cached(
                "SELECT id FROM traces WHERE id>? AND run IS NOT NULL ORDER BY id LIMIT ?",
            )?
            .query_map(params![cursor, BATCH], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for trace in &traces {
            let rows = tx
                .prepare_cached(
                    "SELECT c.call_id,d.preview FROM captured_calls c
                LEFT JOIN conversation_calls d USING(trace_id,call_id) WHERE c.trace_id=?",
                )?
                .query_map([trace], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let previews = rows
                .iter()
                .map(|(call, raw)| {
                    raw.as_deref()
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                        .map(|data| (call, data))
                })
                .collect::<Option<Vec<_>>>();
            match previews {
                Some(previews)
                    if previews
                        .iter()
                        .all(|(_, data)| !super::conversation::needs_tool_preview(data)) =>
                {
                    for (call, data) in previews {
                        super::activity_search::index(&tx, trace, call, &data)?;
                    }
                }
                _ => {
                    if let Some(run) = load_run(&tx, trace)? {
                        super::conversation::index(&tx, &run)?;
                    }
                }
            }
        }
        let next = traces.last().filter(|_| traces.len() == BATCH);
        tx.execute(
            "UPDATE activity_index_progress SET cursor=? WHERE id=1",
            [next],
        )?;
        tx.commit()?;
        Ok(next.is_some())
    }

    pub(super) fn search_activity(&self, filters: &BTreeMap<String, String>) -> Result<Value> {
        super::activity_search::search(&self.0, filters)
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
        routing_runs: &[Run],
        reports: &[replay::Replay],
        activations: &[activation::Report],
        now: i64,
    ) -> Result<()> {
        let tx = self.0.transaction()?;
        for (run, workflow) in runs
            .iter()
            .map(|r| (r, "returns-demo"))
            .chain(routing_runs.iter().map(|r| (r, "routing-prefill")))
        {
            nexusweave::validate(run)?;
            save_trace(
                &tx,
                &run.id,
                Some(run),
                None,
                &BTreeMap::from([("service.name".into(), workflow.into())]),
                now,
            )?;
        }
        for report in reports {
            replay::validate_report(report, runs)?;
            save_report(&tx, report)?;
        }
        for activation in activations {
            activation.validate(runs)?;
            save_report(&tx, activation)?;
        }
        for run in runs.iter().chain(routing_runs) {
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
        if version == 35 {
            return Ok(());
        }
        if !(0..35).contains(&version) {
            return Err("unsupported sidecar database version".into());
        }
        if version < 16 {
            self.upgrade_legacy_observations(version)?;
        }
        let mut after = String::new();
        loop {
            let tx = self.0.transaction()?;
            let mut stmt = tx.prepare(
                "SELECT t.id, EXISTS (
                SELECT 1 FROM observations o WHERE o.trace_id=t.id AND o.kind='prefill'
            ) AS has_ranking, EXISTS(SELECT 1 FROM observations o WHERE o.trace_id=t.id AND o.kind='conflict')
               AND EXISTS(SELECT 1 FROM observations o WHERE o.trace_id=t.id AND o.kind='replay') AS has_conflict_trials,
                EXISTS(SELECT 1 FROM observations o WHERE o.trace_id=t.id AND o.kind='prefill'
                    AND json_type(o.detail,'$.ranked[0].opening_changed') IN ('true','false')) AS has_opening,
                instr(t.run,'\"candidate_probes\"')>0 AS has_candidates,
                (?2<31 AND EXISTS(SELECT 1 FROM reports p WHERE p.trace_id=t.id AND p.kind='activation-reports'
                    AND json_extract(p.body,'$.kind')='source_visibility' AND json_array_length(p.body,'$.continuations')>0)) AS has_masks,
                instr(t.run,'\"sandbox_dispatch\"')>0 AS has_dispatch,
                EXISTS(SELECT 1 FROM reports p WHERE p.trace_id=t.id AND p.kind='replay-reports'
                    AND json_extract(p.body,'$.baseline_summary.matched')=0) AS has_unmatched_baseline,
                (EXISTS(SELECT 1 FROM observations o WHERE o.trace_id=t.id AND o.kind='duplicate')
                    AND instr(t.run,'\"text\":\"(no content)')>0) AS has_tool_placeholder
            FROM traces t WHERE t.id>?1 AND t.run IS NOT NULL AND (?2<23 OR (?2<25 AND has_conflict_trials)
                OR (?2<26 AND has_opening) OR (?2<30 AND has_candidates) OR has_masks OR has_dispatch OR has_unmatched_baseline OR has_tool_placeholder)
            ORDER BY t.id LIMIT 16",
            )?;
            let ids = stmt
                .query_map(params![after, version], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, bool>(4)?,
                        r.get::<_, bool>(5)?,
                        r.get::<_, bool>(6)?,
                        r.get::<_, bool>(7)?,
                        r.get::<_, bool>(8)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            let Some(last) = ids.last() else { break };
            for (
                id,
                has_ranking,
                has_conflict_trials,
                has_opening,
                has_candidates,
                has_masks,
                has_dispatch,
                has_unmatched_baseline,
                has_tool_placeholder,
            ) in &ids
            {
                let run = load_run(&tx, id)?;
                if let Some(run) = run {
                    if version < 20 {
                        super::capture::replace(&tx, id, Some(&run))?;
                    } else if version < 23 {
                        super::conversation::index(&tx, &run)?;
                    }
                    if (20..30).contains(&version) && *has_candidates {
                        tx.execute("DELETE FROM prefix_checks WHERE trace_id=?", [id])?;
                        for call in &run.calls {
                            super::prefix_patterns::index(&tx, &run, call)?;
                        }
                    }
                    if (version < 22 && *has_ranking)
                        || (version < 25 && *has_conflict_trials)
                        || (version < 26 && *has_opening)
                        || *has_masks
                        || *has_dispatch
                        || *has_unmatched_baseline
                        || *has_tool_placeholder
                    {
                        refresh_findings(&tx, &run)?;
                    }
                }
            }
            tx.commit()?;
            after.clone_from(&last.0);
        }
        self.0.pragma_update(None, "user_version", 35)?;
        Ok(())
    }

    fn upgrade_legacy_observations(&mut self, version: i64) -> Result<()> {
        let mut after = String::new();
        loop {
            let ids = {
                let mut stmt = self.0.prepare(
                    "SELECT id FROM traces WHERE id>?1 AND run IS NOT NULL AND
                      (?2<8 OR EXISTS (SELECT 1 FROM observations o WHERE o.trace_id=traces.id AND o.kind IN ('token','shadow','prefill'))
                       OR (?2<13 AND EXISTS (SELECT 1 FROM reports p WHERE p.trace_id=traces.id AND p.kind='activation-reports'))
                       OR instr(run,'shadow_probes')>0 OR EXISTS (SELECT 1 FROM reports p WHERE p.trace_id=traces.id AND p.kind='replay-reports') OR (?2=8 AND (instr(raw,?3)>0 OR (raw IS NULL AND instr(run,'prefill_probe')>0)))
                       OR (?2 IN (9,10) AND (instr(raw,'routing_test')>0 OR instr(run,'routing_test')>0)))
                      ORDER BY id LIMIT 16",
                )?;
                let rows = stmt
                    .query_map(params![after, version, replay::prefill::ATTRIBUTE], |row| {
                        row.get::<_, String>(0)
                    })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let Some(last) = ids.last() else { break };
            let tx = self.0.transaction()?;
            for id in &ids {
                // Older normalized calls omitted prefill reports even when the retained span had one.
                let saved: Option<String> = tx.query_row(
                    "SELECT CASE WHEN instr(raw,?2)>0 THEN raw END FROM traces WHERE id=?1",
                    params![id, replay::prefill::ATTRIBUTE],
                    |row| row.get(0),
                )?;
                if let (Some(raw), Some(trace_id)) = (saved, id.strip_prefix("otlp:")) {
                    let stored: otlp::StoredTrace = serde_json::from_str(&raw)?;
                    if let Some(run) = stored.normalize(trace_id)? {
                        tx.execute(
                            "UPDATE traces SET run=?1 WHERE id=?2",
                            params![serde_json::to_string(&run)?, id],
                        )?;
                    }
                }
                if let Some(run) = load_run(&tx, id)? {
                    refresh_findings(&tx, &run)?;
                }
            }
            tx.commit()?;
            after.clone_from(last);
        }
        self.0.pragma_update(None, "user_version", 16)?;
        Ok(())
    }

    pub fn trace(&self, id: &str) -> Result<Option<Run>> {
        load_run(&self.0, id)
    }

    pub fn replays(&self, id: &str) -> Result<Vec<replay::Replay>> {
        load_reports(&self.0, id)
    }

    pub fn position_plan(
        &self,
        filters: &BTreeMap<String, String>,
    ) -> Result<replay::position::Plan> {
        let transaction = self.0.unchecked_transaction()?;
        let plan = self.resolve_position_plan(filters)?;
        transaction.commit()?;
        Ok(plan)
    }

    fn resolve_position_plan(
        &self,
        filters: &BTreeMap<String, String>,
    ) -> Result<replay::position::Plan> {
        let row = self.selected_position_pair(filters)?;
        super::position_plan::prepare(self, &row)
    }

    fn selected_position_pair(&self, filters: &BTreeMap<String, String>) -> Result<Value> {
        for key in ["factor", "group", "baseline_trials", "edit_trials", "pair"] {
            if filters.get(key).is_none_or(String::is_empty) {
                return Err(format!("position plan requires {key}").into());
            }
        }
        let listed = self.list("replay-variations", filters)?;
        let row = listed["rows"]
            .as_array()
            .and_then(|rows| rows.first())
            .filter(|row| row["position_pair"].as_str() == Some(filters["pair"].as_str()))
            .ok_or("this pair of measured requests is unavailable in the selected scope")?;
        Ok(row.clone())
    }

    pub(super) fn attach_position(
        &self,
        submitted: super::position_plan::Submission,
    ) -> std::result::Result<bool, Rejected> {
        let filters = submitted.filters().map_err(|e| (400, e.to_string()))?;
        let tx = self.0.unchecked_transaction().map_err(storage)?;
        let plan = self
            .resolve_position_plan(&filters)
            .map_err(|e| (400, e.to_string()))?;
        let mut result = submitted.result;
        let [present, replaced] = plan.verify(&result).map_err(|e| (400, e))?;
        result.trials.sort_by(|a, b| {
            (
                &a.location,
                &a.replay_id,
                &a.context_id,
                a.condition.position,
                a.condition.source_replaced,
            )
                .cmp(&(
                    &b.location,
                    &b.replay_id,
                    &b.context_id,
                    b.condition.position,
                    b.condition.source_replaced,
                ))
        });
        let mut canonical = serde_json::to_value(&result).map_err(storage)?;
        canonical.sort_all_objects();
        let id = super::position_plan::digest(&canonical.to_string());
        let body = serde_json::to_string(&result).map_err(storage)?;
        let present_already: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM position_results WHERE pair=? AND id=?)",
                params![plan.pair, id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if present_already {
            return Ok(false);
        }
        let [a, b] = plan.backgrounds.each_ref().map(|bg| &bg.location.run_id);
        for trace in [a, b] {
            let bytes: usize = tx
                .query_row(
                    "SELECT coalesce(sum(length(cast(body AS blob))),0) FROM position_results
                WHERE first_trace=?1 OR second_trace=?1",
                    [trace],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if bytes + body.len() > MAX_REPORT_BYTES_PER_TRACE {
                return Err((413, "position results exceed 4 MiB for a trace".into()));
            }
        }
        let summary =
            json!({"id":id,"runner_id":result.runner_id,"present":present,"replaced":replaced});
        tx.execute("INSERT INTO position_results(pair,id,first_trace,second_trace,body,summary) VALUES(?,?,?,?,?,?)",
            params![plan.pair,id,a,b,body,summary.to_string()]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(true)
    }

    pub fn position_results(&self, filters: &BTreeMap<String, String>) -> Result<Value> {
        let tx = self.0.unchecked_transaction()?;
        self.selected_position_pair(filters)?;
        let pair = &filters["pair"];
        let data = if let Some(id) = filters.get("id") {
            let body: String = tx.query_row(
                "SELECT body FROM position_results WHERE pair=? AND id=?",
                [pair, id],
                |r| r.get(0),
            )?;
            serde_json::from_str(&body)?
        } else {
            let offset = filters
                .get("report_offset")
                .map(|v| v.parse::<u32>())
                .transpose()?
                .unwrap_or(0);
            let mut stmt = tx.prepare("SELECT summary FROM position_results WHERE pair=? ORDER BY rowid DESC LIMIT ? OFFSET ?")?;
            let mut rows: Vec<Value> = stmt
                .query_map(params![pair, PAGE + 1, offset], |r| r.get::<_, String>(0))?
                .map(|r| Ok(serde_json::from_str(&r?)?))
                .collect::<Result<_>>()?;
            let next = (rows.len() > PAGE).then(|| offset.saturating_add(PAGE as u32));
            rows.truncate(PAGE);
            json!({"rows":rows,"next_offset":next})
        };
        tx.commit()?;
        Ok(data)
    }

    pub fn token_path(&self, filters: &BTreeMap<String, String>) -> Result<Value> {
        let mut sql = "SELECT p.body,t.run FROM reports p JOIN traces t ON t.id=p.trace_id
            JOIN observations o ON o.trace_id=p.trace_id AND o.call_id=p.call_id
            WHERE p.kind='activation-reports' AND o.kind='token' AND p.trace_id=? AND p.call_id=? AND p.decision_id=? AND p.id=?
            AND o.group_id=? AND json_extract(o.detail,'$.report_id')=p.id
            AND json_extract(o.detail,'$.location.decision_id')=p.decision_id".to_owned();
        let mut args = Vec::new();
        for key in ["trace", "call", "decision", "report", "group"] {
            args.push(super::required(filters, key)?.to_owned());
        }
        if let Some(factor) = filters.get("factor") {
            sql.push_str(" AND json_extract(o.detail,'$.factor')=?");
            args.push(factor.clone());
        }
        append_scope(&mut sql, &mut args, filters)?;
        append_window(&mut sql, &mut args, "o.observed", filters)?;
        let (body, run): (String, String) = self
            .0
            .query_row(&sql, params_from_iter(args), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?
            .ok_or("Token measurements are unavailable in this scope.")?;
        let report: activation::Report = serde_json::from_str(&body)?;
        let run: Run = serde_json::from_str(&run)?;
        let call = run
            .calls
            .iter()
            .find(|c| c.id == report.location().call_id)
            .ok_or("Missing measured call.")?;
        let offset = filters
            .get("offset")
            .map(|s| s.parse::<usize>())
            .transpose()?
            .unwrap_or(0);
        let block = super::required(filters, "block")?.parse::<usize>()?;
        let request = serde_json::from_value(
            json!({"version":1,"report_id":report.id(),"location":report.location(),
            "block":block,"offset":offset,"limit":16,"include_excerpts":true,"include_edits":true}),
        )?;
        let mut page = activation::output_scores::query(&request, &[report], true)?;
        let edits = page["edits"]
            .as_array_mut()
            .ok_or("Missing source edits.")?;
        let contexts: BTreeMap<_, _> = call.context.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut tested = BTreeSet::new();
        for edit in edits {
            let item = contexts
                .get(edit["context_id"].as_str().unwrap_or_default())
                .ok_or("Missing tested source.")?;
            edit["source"] = findings::source(item);
            tested.insert(item.id.as_str());
        }
        page["captured_sources"] = json!(contexts.len());
        page["untested_examples"] = json!(contexts
            .values()
            .filter(|c| !tested.contains(c.id.as_str()))
            .take(12)
            .map(|c| findings::source(c))
            .collect::<Vec<_>>());
        Ok(page)
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
        let data: String = self.0.query_row(
            super::history::PRIOR_EDITS,
            rusqlite::params![partition, current],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn prior_dispatch(
        &self,
        partition: &str,
        current: &str,
        scope: Option<&nexusweave::DecisionScope>,
    ) -> Result<Value> {
        let data: String = self.0.query_row(
            super::history::PRIOR_DISPATCH,
            params![
                partition,
                current,
                scope.map(|s| &s.key),
                scope.map(|s| &s.scope)
            ],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn prior_replay(&self, example: &Value) -> Result<Option<replay::Replay>> {
        let body: Option<String> = self.0.query_row(
            "SELECT body FROM reports WHERE trace_id=?1 AND kind='replay-reports' AND call_id=?2 AND decision_id=?3 AND id=?4",
            params![example["trace"].as_str(), example["call"].as_str(), example["decision"].as_str(), example["report"].as_str()],
            |row| row.get(0),
        ).optional()?;
        Ok(body.as_deref().map(serde_json::from_str).transpose()?)
    }

    pub fn target_check(&self, fields: &BTreeMap<String, String>) -> Result<Option<Value>> {
        let trace = super::required(fields, "trace")?;
        let call_id = super::required(fields, "call")?;
        let report = super::required(fields, "report")?;
        // Both the indexed witness and its raw trials must come from the same capture.
        let _snapshot = self.0.unchecked_transaction()?;
        let examples = self.list("target-examples", fields)?;
        let Some(example) = examples["rows"].get(0) else {
            return Ok(None);
        };
        let Some(run) = self.trace(trace)? else {
            return Ok(None);
        };
        let Some(call) = run.calls.iter().find(|call| call.id == call_id) else {
            return Ok(None);
        };
        let probes = replay::shadow::recorded(trace, call);
        let raw = call
            .shadow_probes
            .as_ref()
            .and_then(|c| c.reports.as_ref())
            .and_then(|reports| reports.iter().find(|r| r.id == report));
        Ok(super::targets::checks(call, &probes).into_iter()
            .find(|c| c["report_id"] == report).map(|check| json!({
                "example":example,"check":check,"trials":raw,"question":super::capture::last_user_text(call)
            })))
    }

    pub fn list(&self, view: &str, filters: &BTreeMap<String, String>) -> Result<Value> {
        // A selected conflict reads its observation, source fields and trial examples across
        // statements. A late span or attached report must appear in all of them together.
        let _snapshot = (view == "findings"
            && filters.get("group").is_some_and(|id| !id.is_empty()))
        .then(|| self.0.unchecked_transaction())
        .transpose()?;
        let mut sql = if view == "threads" {
            // A thread key exists only for a normalized run. The unscoped list can
            // group keys and timestamps in the index without reading the trace body.
            let scope_needs_columns = ["org", "user", "workflow", "meta_key"]
                .iter()
                .any(|key| filters.get(*key).is_some_and(|value| !value.is_empty()));
            let mut selected = "WITH selected AS (SELECT t.thread_key,t.start FROM traces t WHERE t.thread_key IS NOT NULL".to_owned();
            if scope_needs_columns {
                selected.push_str(" AND t.run IS NOT NULL");
            }
            selected
        } else {
            "WITH selected AS (SELECT t.id,t.start,t.org,t.user,t.thread,t.thread_key,t.workflow,t.source,t.calls,t.with_inputs,t.metadata FROM traces t WHERE t.run IS NOT NULL".to_owned()
        };
        let mut args = Vec::new();
        append_scope(&mut sql, &mut args, filters)?;
        if matches!(view, "traces" | "threads") {
            append_window(&mut sql, &mut args, "t.start", filters)?;
        }
        let thread_name = filters
            .get("name")
            .map(|name| name.trim())
            .filter(|name| view == "threads" && !name.is_empty());
        if let Some(name) = thread_name {
            if name.chars().count() > 160 {
                return Err("conversation search is limited to 160 characters".into());
            }
            sql.push_str(" AND instr(lower(json_extract(t.thread_key,'$[3]')),lower(?))>0");
            args.push(name.to_owned());
        }
        sql.push(')');
        let mut offset = filters
            .get("offset")
            .map(|offset| offset.parse::<u32>())
            .transpose()?
            .unwrap_or(0);
        let page_size = if view == "thread-calls"
            && !filters.contains_key("focus_trace")
            && !filters.contains_key("focus_call")
        {
            filters
                .get("limit")
                .map(|limit| limit.parse::<usize>())
                .transpose()?
                .unwrap_or(PAGE)
                .clamp(1, PAGE)
        } else {
            PAGE
        };
        if matches!(
            view,
            "findings"
                | "factors"
                | "impact"
                | "summary"
                | "conversations-without-findings"
                | "opening-coverage"
                | "opening-sources"
                | "opening-conversations"
                | "conflict-examples"
                | "conflict-conversations"
                | "conflict-coverage"
                | "dispatch-summary"
                | "dispatch-conversations"
                | "dispatch-designs"
                | "outcome-factors"
                | "conversation-outcomes"
                | "proposal-conversations"
                | "occurrences"
                | "order-tests"
                | "replay-variations"
        ) {
            sql.push_str(", events AS (SELECT o.*,t.thread_key,t.thread,t.org,t.user,t.source,t.workflow,json_extract(o.detail,'$.factor') AS factor,
                CASE WHEN o.kind IN ('candidate','token') OR (o.kind='prefill' AND json_extract(o.detail,'$.shifted')=1) OR
                    (o.kind IN ('replay','shadow') AND json_extract(o.detail,'$.effect_pp') != 0) THEN 1 ELSE 0 END AS changed,
                CASE WHEN json_array_length(o.detail,'$.changed_edits')>0
                     THEN 1 ELSE 0 END AS proposal_changed
                FROM observations o JOIN selected t ON t.id=o.trace_id WHERE 1");
            for (key, field) in [
                ("kind", "o.kind"),
                (
                    "routing_input",
                    "json_extract(o.detail,'$.baseline_request_sha256')",
                ),
                ("group", "o.group_id"),
                ("joint_pattern", "json_extract(o.detail,'$.joint_pattern')"),
                ("report", "json_extract(o.detail,'$.report_id')"),
                ("trace", "o.trace_id"),
                ("call", "o.call_id"),
                ("replay", "json_extract(o.detail,'$.measurement.replay_id')"),
                (
                    "decision",
                    "json_extract(o.detail,'$.measurement.location.decision_id')",
                ),
                (
                    "context",
                    "json_extract(o.detail,'$.measurement.context_id')",
                ),
                ("factor", "json_extract(o.detail,'$.factor')"),
                (
                    "routing_design",
                    "json_extract(o.detail,'$.routing_test.design')",
                ),
            ] {
                if let Some(value) = filters.get(key).filter(|value| !value.is_empty()) {
                    sql.push_str(&format!(" AND {field}=?"));
                    args.push(value.clone());
                }
            }
            if view == "replay-variations"
                || (view == "occurrences" && filters.contains_key("joint_pattern"))
            {
                for (key, path) in [
                    ("baseline_trials", "$.measurement.baseline.attempted"),
                    ("edit_trials", "$.measurement.variant.attempted"),
                ] {
                    if let Some(count) = filters.get(key) {
                        let count = count.parse::<u32>()?;
                        sql.push_str(&format!(
                            " AND json_extract(o.detail,'{path}')=CAST(? AS INTEGER)"
                        ));
                        args.push(count.to_string());
                    }
                }
            }
            append_window(&mut sql, &mut args, "o.observed", filters)?;
            sql.push(')');
        }
        if matches!(
            view,
            "conflict-coverage" | "conflict-conversations" | "impact"
        ) {
            sql.push_str(super::conflict::COVERAGE);
        }
        if matches!(
            view,
            "impact" | "opening-coverage" | "opening-sources" | "opening-conversations"
        ) {
            sql.push_str(super::opening::CALLS);
        }
        if matches!(view, "impact" | "conflict-examples") {
            sql.push_str(super::conflict::PATTERNS);
        }
        if matches!(view, "order-tests" | "factors") {
            sql.push_str(super::order::MATCHED);
        }
        if matches!(view, "replay-variations" | "factors") {
            sql.push_str(super::variation::BY_INPUT);
        }
        if view == "factors" {
            sql.push_str(super::variation::TESTED_POSITIONS);
            super::prefill::append_routing(&mut sql, false);
        }
        if matches!(view, "impact" | "outcome-factors" | "conversation-outcomes") {
            sql.push_str(super::outcomes::CLASSIFIED);
        }
        if matches!(
            view,
            "impact" | "proposal-conversations" | "conversations-without-findings"
        ) {
            sql.push_str(super::outcomes::THREAD_OUTCOMES);
        }
        if matches!(view, "impact" | "conversations-without-findings") {
            sql.push_str(", scoped_calls AS (SELECT t.thread_key,c.trace_id,c.call_id,c.started,c.with_text,c.screen IS NOT NULL AS flagged
                FROM selected t JOIN captured_calls c ON c.trace_id=t.id WHERE t.thread_key IS NOT NULL");
            for (key, column) in [("trace", "c.trace_id"), ("call", "c.call_id")] {
                if let Some(value) = filters.get(key).filter(|s| !s.is_empty()) {
                    sql.push_str(&format!(" AND {column}=?"));
                    args.push(value.clone());
                }
            }
            append_window(&mut sql, &mut args, "c.started", filters)?;
            sql.push_str(super::capture::BY_CONVERSATION);
        }
        if matches!(view, "screened-pairs" | "screened-pair-examples") {
            sql.push_str(super::screened::SCOPED);
            for (key, column) in [("trace", "p.trace_id"), ("call", "p.call_id")] {
                if let Some(value) = filters.get(key).filter(|v| !v.is_empty()) {
                    sql.push_str(&format!(" AND {column}=?"));
                    args.push(value.clone());
                }
            }
            if view == "screened-pair-examples" || filters.contains_key("pattern") {
                sql.push_str(" AND p.pattern=? AND p.relation=?");
                args.push(super::required(filters, "pattern")?.to_owned());
                args.push(super::required(filters, "relation")?.to_owned());
            }
            append_window(&mut sql, &mut args, "c.started", filters)?;
            sql.push_str(if view == "screened-pair-examples" {
                super::screened::TESTED
            } else {
                ")"
            });
        }
        if matches!(
            view,
            "prefix-groups"
                | "prefix-sources"
                | "prefix-references"
                | "prefix-cases"
                | "prefix-reference-pairs"
                | "prefix-contrasts"
        ) {
            sql.push_str(super::prefix_patterns::SCOPED);
            if view != "prefix-groups" || filters.contains_key("design") {
                sql.push_str(" AND p.design=?");
                args.push(super::required(filters, "design")?.to_owned());
            }
            append_window(&mut sql, &mut args, "c.started", filters)?;
            sql.push(')');
            let only_pending = filters.get("needs_trial").is_some_and(|v| v == "1");
            if view == "prefix-groups" || only_pending {
                sql.push_str(super::prefix_patterns::TRIAL_COVERAGE);
                sql.push_str(", checks AS (SELECT * FROM coverage p WHERE 1");
                if only_pending {
                    sql.push_str(" AND needs_trial");
                }
            } else {
                sql.push_str(", checks AS (SELECT * FROM scoped_checks p WHERE 1");
            }
            if matches!(
                view,
                "prefix-cases" | "prefix-references" | "prefix-sources"
            ) {
                for (key, column) in [
                    ("ref_outcome", "reference_outcome"),
                    ("ref_set", "reference_set"),
                    ("ref_prediction", "reference_prediction"),
                ] {
                    if let Some(value) = filters.get(key) {
                        sql.push_str(&format!(" AND p.choices->>'{column}'=?"));
                        args.push(value.clone());
                    }
                }
            }
            if view == "prefix-cases" {
                if let Some(key) = filters.get("ref_edit") {
                    sql.push_str(" AND p.setup=? AND EXISTS (SELECT 1 FROM json_each(p.choices,'$.reference_edits') e
                        WHERE e.value->>'key'=? AND e.value->>'outcome'=? AND e.key+1=CAST(? AS INTEGER))");
                    args.extend([
                        super::required(filters, "pair_setup")?.to_owned(),
                        key.clone(),
                        super::required(filters, "edit_outcome")?.to_owned(),
                        super::required(filters, "edit_index")?
                            .parse::<usize>()?
                            .to_string(),
                    ]);
                }
                if filters.get("reference").is_some_and(|v| v == "1") {
                    sql.push_str(" AND p.choices->>'reference_set' IS NOT NULL");
                }
                for (key, column) in [
                    ("verdict", "p.verdict"),
                    ("trial_key", "p.choices->>'trial_key'"),
                    ("source", "p.source"),
                    ("trace", "p.trace_id"),
                    ("call", "p.call_id"),
                    ("report", "p.report"),
                ] {
                    if let Some(value) = filters.get(key) {
                        sql.push_str(&format!(" AND {column}=?"));
                        args.push(value.clone());
                    }
                }
                if filters.get("tested").is_some_and(|v| v == "1") {
                    sql.push_str(
                        " AND p.verdict IN ('confirmed','disagreed','mixed','unresolved')",
                    );
                }
                if filters.get("unnamed").is_some_and(|v| v == "1") {
                    sql.push_str(" AND p.source IS NULL");
                }
            }
            sql.push(')');
        }
        if matches!(view, "target-patterns" | "target-examples") {
            sql.push_str(super::target_patterns::SCOPED);
            if view == "target-examples" || filters.contains_key("design") {
                sql.push_str(" AND x.design=?");
                args.push(super::required(filters, "design")?.to_owned());
            }
            append_window(&mut sql, &mut args, "c.started", filters)?;
            sql.push_str(super::target_patterns::CONVERSATIONS);
        }
        if matches!(
            view,
            "ranking-results" | "ranking-examples" | "ranking-reference" | "ranking-review"
        ) {
            sql.push_str(
                ", rankings AS (SELECT o.trace_id,o.call_id,o.group_id,o.observed,t.thread,
                json_extract(o.detail,'$.selector_check.result') AS result,
                json_extract(o.detail,'$.selector_check.same_edit') AS same_edit,
                json_extract(o.detail,'$.selector_check.reference') AS reference,
                json_extract(o.detail,'$.selector_check.rank_review.status') AS review,
                json_extract(o.detail,'$.ranked[0].source') AS source
                FROM observations o JOIN selected t ON t.id=o.trace_id
                WHERE o.kind='prefill' AND json_type(o.detail,'$.selector_check')='object'",
            );
            append_window(&mut sql, &mut args, "o.observed", filters)?;
            sql.push(')');
        }
        let cohort = view == "outcome-factors"
            || (view == "conversation-outcomes" && filters.contains_key("level"));
        let edited = filters.get("by").is_some_and(|s| s == "edit");
        if edited && (!cohort || filters.get("level").is_none_or(|level| level != "1")) {
            return Err("edited source groups require proposal evidence (level=1)".into());
        }
        let members = view == "outcome-factors"
            || (cohort && (edited || filters.contains_key("cohort_factor")));
        if cohort {
            let level = super::required(filters, "level")?;
            if !matches!(level, "1" | "2" | "3" | "4" | "5") {
                return Err("choose an evidence level from 1 to 5".into());
            }
            sql.push_str(super::outcomes::COHORT);
            args.push(level.to_owned());
            if members {
                sql.push_str(if edited {
                    super::outcomes::EDIT_MEMBERS
                } else {
                    super::outcomes::SOURCE_MEMBERS
                });
            }
        }
        match view {
            "source-history" => {
                // The graph identifies a named source by producer, organization, workflow, ID, revision and role.
                let key = super::required(filters, "source_key")?;
                let parts: [Option<String>; 6] = serde_json::from_str(key)?;
                if parts[3].as_deref().is_none_or(str::is_empty) {
                    return Err("source history requires a named source".into());
                }
                args.push(key.to_owned());
                sql.push_str(", identity AS (SELECT ? AS k), calls AS (
                    SELECT o.trace_id,o.call_id,t.thread_key,t.thread,t.user,c.started,
                        count(*) AS observations,json_group_array(DISTINCT o.kind ORDER BY o.kind) AS kinds FROM identity JOIN selected t
                        ON t.source IS json_extract(k,'$[0]') AND t.org IS json_extract(k,'$[1]')
                        AND t.workflow IS json_extract(k,'$[2]')
                    JOIN observations o ON o.trace_id=t.id
                    JOIN captured_calls c ON c.trace_id=o.trace_id AND c.call_id=o.call_id
                    WHERE EXISTS (SELECT 1 FROM json_each(o.detail,'$.sources') s
                        WHERE s.value->>'source'=json_extract(k,'$[3]')
                        AND s.value->>'version' IS json_extract(k,'$[4]') AND s.value->>'role' IS json_extract(k,'$[5]'))");
                append_window(&mut sql, &mut args, "o.observed", filters)?;
                if let Some(trace) = filters.get("unthreaded_trace") {
                    sql.push_str(" AND t.thread_key IS NULL AND t.id=?");
                    args.push(trace.clone());
                }
                sql.push_str(" GROUP BY o.trace_id,o.call_id)");
                if filters.get("thread").is_some_and(|s| !s.is_empty()) || filters.contains_key("unthreaded_trace") {
                    sql.push_str(" SELECT json_object('trace',trace_id,'call',call_id,'thread',thread,'user',user,'turn',json_extract(d.preview,'$.turn'),
                        'started',started,'observations',observations,'kinds',json(kinds)) FROM calls
                        LEFT JOIN conversation_calls d USING(trace_id,call_id)
                        ORDER BY started IS NULL,started,d.nanos,trace_id,d.ordinal,call_id LIMIT ? OFFSET ?");
                } else {
                    sql.push_str(", conversations AS (SELECT thread_key,thread,user,
                        CASE WHEN thread_key IS NULL THEN trace_id END AS unthreaded_trace,
                        count(*) AS calls,max(started) AS latest FROM calls GROUP BY thread_key,unthreaded_trace)
                        SELECT json_object('thread_key',thread_key,'thread',thread,'user',user,'unthreaded_trace',unthreaded_trace,
                        'calls',calls,'total_calls',sum(calls) OVER (),'total_threads',count(thread_key) OVER (),
                        'unthreaded_calls',sum(CASE WHEN thread_key IS NULL THEN calls ELSE 0 END) OVER ())
                        FROM conversations ORDER BY calls DESC,latest DESC,thread_key,unthreaded_trace LIMIT ? OFFSET ?");
                }
            }
            "ranking-results" => sql.push_str(" SELECT json_object('result',result,'calls',count(*),
                'same_edit',sum(same_edit)) FROM rankings GROUP BY result ORDER BY result LIMIT ? OFFSET ?"),
            "ranking-review" => sql.push_str(" SELECT json_object('review',review,'calls',count(*)) FROM rankings
                GROUP BY review ORDER BY review LIMIT ? OFFSET ?"),
            "ranking-reference" => sql.push_str(" SELECT json_object('reference',json(reference),'calls',count(*))
                FROM rankings WHERE reference IS NOT NULL GROUP BY reference ORDER BY reference LIMIT ? OFFSET ?"),
            "ranking-examples" => {
                sql.push_str(" SELECT json_object('trace',trace_id,'call',call_id,'group',group_id,'thread',thread,
                    'source',source,'result',result,'same_edit',same_edit,'reference',json(reference),'review',review) FROM rankings WHERE 1");
                if let Some(review) = filters.get("review") {
                    if !matches!(review.as_str(), "lower_changed" | "top_changed" | "all_held" | "mixed" | "incomplete" | "tied") {
                        return Err("unknown edit ranking review".into());
                    }
                    sql.push_str(" AND review=?");
                    args.push(review.clone());
                }
                if let Some(filter) = filters.get("quality") {
                    let (selector,baseline) = match filter.as_str() {
                        "ranked_lost" => ("ranked",1), "ranked_gained" => ("ranked",0),
                        "first_lost" => ("first",1), "first_gained" => ("first",0),
                        _ => return Err("unknown reference comparison".into()),
                    };
                    sql.push_str(&format!(" AND json_extract(reference,'$.original')={baseline}
                        AND json_extract(reference,'$.{selector}')!={baseline}"));
                }
                if let Some(value) = filters.get("result") {
                    if !matches!(value.as_str(),"both"|"ranked_only"|"first_only"|"neither"|"missing"|"mixed") {
                        return Err("unknown ranking comparison result".into());
                    }
                    sql.push_str(" AND result=?");
                    args.push(value.clone());
                }
                sql.push_str(" ORDER BY observed DESC,trace_id,call_id LIMIT ? OFFSET ?");
            }
            "prefix-groups" => sql.push_str(super::prefix_patterns::GROUPS),
            "prefix-references" => sql.push_str(super::prefix_patterns::REFERENCES),
            "prefix-sources" => sql.push_str(super::prefix_patterns::BREAKDOWN),
            "prefix-cases" => sql.push_str(super::prefix_patterns::CASES),
            "prefix-reference-pairs" => sql.push_str(super::prefix_patterns::REFERENCE_PAIRS),
            "prefix-contrasts" => sql.push_str(super::prefix_patterns::CONTRASTS),
            "target-patterns" => {
                args.push(if filters.contains_key("design") {"0"} else {"2"}.into());
                sql.push_str(super::target_patterns::GROUPS);
            }
            "target-examples" => {
                sql.push_str(super::target_patterns::EXAMPLES);
                if let Some(signal) = filters.get("miss_signal") {
                    if !matches!(signal.as_str(),"opening" | "suffix" | "other" | "varied" | "unscored") {
                        return Err("unknown target miss signal".into());
                    }
                    sql.push_str(" AND c.signal=?");
                    args.push(signal.clone());
                }
                for (key, column) in [("trace","r.trace_id"),("call","r.call_id"),("report","r.report")] {
                    if let Some(value) = filters.get(key) {
                        sql.push_str(&format!(" AND {column}=?"));
                        args.push(value.clone());
                    }
                }
                sql.push_str(super::target_patterns::EXAMPLE_ORDER);
            }
            "screened-pairs" => sql.push_str(super::screened::GROUPS),
            "screened-pair-examples" => sql.push_str(super::screened::EXAMPLES),
            "conversations-without-findings" => sql.push_str(super::capture::PAGE),
            "proposal-conversations" => {
                let result = super::required(filters, "result")?;
                if !matches!(result, "changed_only" | "held_only" | "both" | "no_pair") {
                    return Err("unknown proposal comparison result".into());
                }
                args.push(result.to_owned());
                sql.push_str(super::outcomes::PROPOSAL_CONVERSATIONS);
            }
            "outcome-factors" => sql.push_str(super::outcomes::FACTORS),
            "conversation-outcomes" => {
                sql.push_str(", ranked AS (SELECT *,row_number() OVER (
                    PARTITION BY thread_key ORDER BY level,observed DESC,trace_id,call_id,n");
                sql.push_str(if members { ",member_id) AS choice FROM members" } else if cohort {
                    ") AS choice FROM cohort"
                } else { ") AS choice FROM classified" });
                if let Some(factor) = filters.get("cohort_factor") {
                    if !cohort {
                        return Err("select an evidence level before a cohort factor".into());
                    }
                    sql.push_str(" WHERE member_id=?");
                    args.push(factor.clone());
                }
                sql.push_str("), page AS (SELECT * FROM ranked WHERE choice=1");
                sql.push_str(" ORDER BY level,observed DESC,thread_key LIMIT ? OFFSET ?)
                    SELECT json_object('thread',t.thread,'user',t.user,'org',t.org,'level',p.level,
                        'kind',o.kind,");
                if members {
                    sql.push_str("'witness',json(p.witness),'sources',json(p.member_sources),");
                } else {
                    sql.push_str("'sources',json_extract(detail,'$.sources'),");
                }
                sql.push_str("'observed',p.observed,
                        'factor',json_extract(detail,'$.factor'),'group',o.group_id,'trace',p.trace_id,'call',p.call_id,
                        'replay',json_extract(detail,'$.measurement.replay_id'),'report',json_extract(detail,'$.report_id'),
                        'context',json_extract(detail,'$.measurement.context_id'),'decision',json_extract(detail,'$.measurement.location.decision_id'),
                        'unchanged_pair',json(CASE WHEN json_type(detail,'$.held_witness')='object' THEN 'true' ELSE 'false' END))
                    FROM page p JOIN observations o ON o.trace_id=p.trace_id AND o.n=p.n
                    JOIN traces t ON t.id=p.trace_id ORDER BY p.level,p.observed DESC,p.thread_key");
            }
            "opening-coverage" | "opening-sources" | "opening-conversations" => {
                for key in ["group", "factor"] {
                    if super::required(filters,key)?.is_empty() {return Err(format!("missing {key}").into());}
                }
                if view == "opening-coverage" {
                    sql.push_str(super::opening::SUMMARY);
                    sql.push_str(" LIMIT ? OFFSET ?");
                } else {
                    sql.push_str(super::opening::ARMS);
                    if view == "opening-sources" {
                        sql.push_str(super::opening::SOURCES);
                    } else {
                        sql.push_str(super::opening::EXAMPLES);
                        if let Some(slot) = filters.get("slot") {
                            sql.push_str(" AND slot=?");
                            args.push(slot.clone());
                        }
                        sql.push_str(super::opening::PAGE);
                    }
                }
            }
            "conflict-examples" => {
                for key in ["group", "factor", "design"] {
                    if super::required(filters,key)?.is_empty() { return Err(format!("missing {key}").into()); }
                }
                args.push(filters["design"].clone());
                sql.push_str(super::conflict::EXAMPLES);
            }
            "dispatch-designs" => sql.push_str(super::dispatch::DESIGNS),
            "dispatch-summary" | "dispatch-conversations" => {
                for key in ["group","factor","design"] {
                    if super::required(filters,key)?.is_empty() { return Err(format!("missing {key}").into()); }
                }
                args.push(filters["design"].clone());
                sql.push_str(super::dispatch::COHORT);
                if view == "dispatch-summary" {
                    sql.push_str(super::dispatch::SUMMARY);
                } else {
                    let outcome = super::required(filters,"outcome")?;
                    if !super::dispatch::valid_outcome(outcome) { return Err("unknown dispatch outcome".into()); }
                    args.push(outcome.to_owned());
                    sql.push_str(super::dispatch::EXAMPLES);
                }
            }
            "conflict-coverage" | "conflict-conversations" => {
                for key in ["group","factor"] {
                    if super::required(filters,key)?.is_empty() { return Err(format!("missing {key}").into()); }
                }
                if view=="conflict-coverage" {
                    sql.push_str(super::conflict::SUMMARY);
                    sql.push_str(" LIMIT ? OFFSET ?");
                } else {
                    let coverage=super::required(filters,"coverage")?;
                    if !matches!(coverage,"untested"|"partial"|"complete") { return Err("unknown conflict coverage".into()); }
                    args.push(coverage.to_owned());
                    sql.push_str(super::conflict::CONVERSATIONS);
                }
            }
            "replay-variations" => sql.push_str(super::variation::EXAMPLES),
            "order-tests" => sql.push_str(" SELECT json_object('factor',factor,'design',design,
                'differs',differs,'total',count(*) OVER (),'differing',sum(differs) OVER (),
                'forward',json(forward),'reverse',json(reverse)) FROM pairs
                ORDER BY differs DESC,factor,design LIMIT ? OFFSET ?"),
            "summary" => {
                // The landing page needs only counts; detailed findings load on demand.
                sql.push_str(" SELECT json_object('conversations',count(DISTINCT thread_key),
                    'recommendations',count(DISTINCT group_id),'factors',count(DISTINCT factor),
                    'repeat_groups',count(DISTINCT group_id) FILTER (WHERE kind='duplicate'),
                    'themes',json((SELECT json_group_array(json_object('kind',kind,'conversations',conversations,'groups',groups))
                        FROM (SELECT kind,count(DISTINCT thread_key) AS conversations,count(DISTINCT group_id) AS groups
                            FROM events GROUP BY kind ORDER BY conversations DESC,groups DESC,kind LIMIT 3))),
                    'captured_conversations',(SELECT count(DISTINCT t.thread_key)
                        FROM selected t JOIN captured_calls c ON c.trace_id=t.id WHERE t.thread_key IS NOT NULL");
                for (key, column) in [("trace", "c.trace_id"), ("call", "c.call_id")] {
                    if let Some(value) = filters.get(key).filter(|s| !s.is_empty()) {
                        sql.push_str(&format!(" AND {column}=?"));
                        args.push(value.clone());
                    }
                }
                append_window(&mut sql, &mut args, "c.started", filters)?;
                sql.push_str(")) FROM events");
                let row: String = self.0.query_row(&sql, params_from_iter(args), |row| row.get(0))?;
                return Ok(serde_json::from_str(&row)?);
            }
            "impact" => {
                sql.push_str(" SELECT json_object('opening_gap',json((");
                sql.push_str(super::opening::SUMMARY);
                sql.push_str(" WHERE affected>0 ORDER BY affected DESC,inputs DESC,latest DESC,group_id,factor LIMIT 1)),
                    'conflict_pattern',json((");
                sql.push_str(super::conflict::TOP_PATTERN);
                sql.push_str(")),'conflict_gap',json((");
                sql.push_str(super::conflict::SUMMARY);
                sql.push_str(" WHERE conversations>=2 AND (complete+partial)>0 AND (untested+partial)>0
                    ORDER BY untested DESC,partial DESC,conversations DESC,latest DESC,group_id,factor LIMIT 1)),
                    'conversations',count(DISTINCT thread_key),
                    'recommendations',count(DISTINCT group_id),
                    'ranked_calls',count(*) FILTER (WHERE kind='prefill' AND json_type(detail,'$.selector_check')='object'),
                    'lower_edit_changes',count(*) FILTER (WHERE kind='prefill' AND json_extract(detail,'$.selector_check.rank_review.status')='lower_changed'),
                    'lower_edit_conversations',count(DISTINCT thread_key) FILTER (WHERE kind='prefill' AND json_extract(detail,'$.selector_check.rank_review.status')='lower_changed'),
                    'ranked_reference_losses',count(*) FILTER (WHERE kind='prefill'
                        AND json_extract(detail,'$.selector_check.reference.original')=1 AND json_extract(detail,'$.selector_check.reference.ranked')=0),
                    'captured_conversations',(SELECT count(*) FROM captured_threads),
                    'sandbox_dispatch_conversations',count(DISTINCT thread_key)
                        FILTER (WHERE json_extract(detail,'$.sandbox_dispatch.protected')=1),
                    'sandbox_regression_conversations',count(DISTINCT thread_key)
                        FILTER (WHERE json_extract(detail,'$.sandbox_dispatch.outcome')='matched_without_check'),
                    'sandbox_tested_conversations',count(DISTINCT thread_key)
                        FILTER (WHERE json_type(detail,'$.sandbox_dispatch')='object'),
                    'without_findings',(SELECT count(*) FROM without_findings),
                    'without_findings_with_screen_suggestion',(SELECT count(*) FROM without_findings WHERE flagged),
                    'changed',count(DISTINCT CASE WHEN changed=1 OR proposal_changed=1 THEN thread_key END),
                    'evidence_counts',(SELECT json_object('proposal',count(*) FILTER (WHERE level=1),'rate',count(*) FILTER (WHERE level=2),
                        'score',count(*) FILTER (WHERE level=3),'tested',count(*) FILTER (WHERE level=4),'inputs',count(*) FILTER (WHERE level=5))
                        FROM (SELECT min(level) AS level FROM classified GROUP BY thread_key)),
                    'replay_outcomes',(SELECT json_object(
                        'changed_only',count(*) FILTER (WHERE shifted AND NOT held),
                        'equal_only',count(*) FILTER (WHERE held AND NOT shifted),
                        'mixed',count(*) FILTER (WHERE shifted AND held),
                        'unmeasured',count(*) FILTER (WHERE NOT shifted AND NOT held))
                        FROM by_thread),
                    'proposal_outcomes',(SELECT json_object(
                        'changed_only',count(*) FILTER (WHERE result='changed_only'),
                        'held_only',count(*) FILTER (WHERE result='held_only'),
                        'both',count(*) FILTER (WHERE result='both'),
                        'no_pair',count(*) FILTER (WHERE result='no_pair'))
                        FROM thread_results),
                    'unlinked_requests',count(DISTINCT CASE WHEN thread_key IS NULL THEN json_array(trace_id,call_id) END),
                    'factors',count(DISTINCT factor)) FROM events");
                let row: String = self.0.query_row(&sql, params_from_iter(args), |row| row.get(0))?;
                return Ok(serde_json::from_str(&row)?);
            }
            "factors" => sql.push_str(", disputed AS (
                SELECT factor,min(json_object('group',group_id,'design',design,'input',input,'metric',metric,
                    'source',(SELECT coalesce(json_extract(s.value,'$.source'),json_extract(s.value,'$.id'))
                        FROM observations o, json_each(o.detail,'$.sources') s
                        WHERE o.trace_id=json_extract(r.example,'$.trace') AND o.call_id=json_extract(r.example,'$.call')
                          AND o.group_id=r.group_id AND json_extract(o.detail,'$.routing_test.design')=r.design
                          AND json_extract(o.detail,'$.baseline_request_sha256')=r.input
                          AND json_extract(s.value,'$.id')=json_extract(o.detail,'$.routing_test.context_id') LIMIT 1),
                    'setup',json_extract(votes,'$.setup'),'changed',json_extract(votes,'$.changed_runs'),
                    'held',json_extract(votes,'$.held_runs'),'missing',json_extract(votes,'$.missing_runs'))) AS test
                FROM (SELECT group_id,design,input,factor,example,
                    CASE WHEN json_type(witnesses,'$.tool') IS NULL THEN 'proposal' ELSE 'tool' END AS metric,
                    coalesce(json_extract(witnesses,'$.tool'),json_extract(witnesses,'$.proposal')) AS votes
                    FROM routing_inputs WHERE witnesses IS NOT NULL) r GROUP BY factor
            ), grouped AS (
                SELECT factor,org,factor IN (SELECT factor FROM variations) AS varies,json_extract(min(detail),'$.sources') AS sources,
                    count(DISTINCT thread_key) AS conversations,count(DISTINCT trace_id) AS requests,
                    count(DISTINCT CASE WHEN changed=1 OR proposal_changed=1 THEN trace_id END) AS changed_requests,
                    count(DISTINCT thread_key) FILTER (WHERE json_extract(detail,'$.sandbox_dispatch.protected')=1) AS sandbox_conversations,
                    count(DISTINCT CASE WHEN kind='prefill' AND json_extract(detail,'$.routing_test.design') IS NOT NULL
                        THEN json_extract(detail,'$.baseline_request_sha256') END) AS routing_inputs,
                    count(DISTINCT group_id) + (factor IN (SELECT factor FROM pairs)) + (factor IN (SELECT factor FROM variations)) AS recommendations,max(observed) AS latest,
                    CASE WHEN factor IN (SELECT factor FROM disputed) THEN -6 WHEN factor IN (SELECT factor FROM pairs WHERE differs) THEN -4 ELSE
                    min(CASE WHEN kind='replacement' THEN -3 WHEN json_extract(detail,'$.joint_pattern')='joint_only' THEN -2 WHEN json_extract(detail,'$.joint_only')=1 THEN -1 WHEN kind='candidate' AND json_extract(detail,'$.baseline')*json_extract(detail,'$.edited')<0 THEN 0
                        WHEN kind='replay' AND changed=1 THEN 1 WHEN kind IN ('shadow','replay') AND (changed OR proposal_changed) THEN 2
                        WHEN json_extract(detail,'$.sandbox_dispatch.outcome')='matched_without_check' THEN 1.5
                        WHEN json_extract(detail,'$.sandbox_dispatch.protected')=1 THEN 2.25
                        WHEN kind='source_mask' AND proposal_changed THEN 2.5 WHEN kind='token' THEN 3
                        WHEN kind='prefill' AND json_extract(detail,'$.routing_test') IS NOT NULL THEN -5
                        WHEN kind='baseline_check' THEN 4.75 WHEN kind='source_gap' THEN 4.5 WHEN kind='prefill' THEN 6 WHEN kind='conflict' THEN 4 WHEN kind='candidate' THEN 5 WHEN kind='duplicate' AND json_extract(detail,'$.dialogue_only')=1 THEN 8 WHEN kind='duplicate' THEN 7 WHEN kind='source_mask' THEN 8.5 ELSE 9 END) END AS signal
                FROM events GROUP BY factor)
                SELECT json_object('id',g.factor,'sources',json(sources),'org',org,'conversations',conversations,
                    'requests',requests,'changed',changed_requests,'recommendations',recommendations,'routing_inputs',routing_inputs,
                    'sandbox_conversations',sandbox_conversations,
                    'disputed_routing',json(d.test),'order_counts',CASE WHEN signal=-4 THEN json((
                       SELECT json_object('changed',sum(differs),'total',count(*)) FROM pairs WHERE factor=g.factor)) END,
                    'position_test',CASE WHEN t.pair IS NOT NULL THEN json_object(
                        'pair',t.pair,'group',t.group_id,'baseline_trials',t.baseline_trials,'edit_trials',t.edit_trials,
                        'result',json(t.summary)) END,
                    'signal',CASE signal WHEN -6 THEN 'routing_repeats' WHEN -5 THEN 'routing' WHEN -4 THEN 'order' WHEN -3 THEN 'replacement' WHEN -2 THEN 'joint_proposal' WHEN -1 THEN 'joint' WHEN 0 THEN 'flip' WHEN 1 THEN CASE WHEN varies THEN 'variation' ELSE 'replay' END WHEN 2 THEN 'shadow' WHEN 3 THEN 'token'
                        WHEN 4.75 THEN 'baseline_check' WHEN 4.5 THEN 'source_gap' WHEN 4 THEN 'conflict' WHEN 5 THEN 'score' WHEN 6 THEN 'prefill' WHEN 7 THEN 'duplicate' WHEN 8 THEN 'dialogue'
                        WHEN 1.5 THEN 'sandbox_regression' WHEN 2.25 THEN 'sandbox_dispatch' WHEN 2.5 THEN 'source_mask' WHEN 8.5 THEN 'source_mask' ELSE 'equal' END)
                FROM grouped g LEFT JOIN disputed d ON d.factor=g.factor LEFT JOIN tested_factors t ON t.factor=g.factor AND t.priority=1
                ORDER BY signal,CASE WHEN signal=1 THEN coalesce(t.isolated,0) END DESC,CASE WHEN signal=1 THEN varies END DESC,conversations DESC,requests DESC,latest DESC,g.factor LIMIT ? OFFSET ?"),
            "occurrences" => {
                let by_thread = filters.get("by_thread").is_some_and(|value| value == "1");
                if by_thread {
                    sql.push_str(", examples AS (SELECT *,row_number() OVER (PARTITION BY thread_key
                        ORDER BY observed DESC,trace_id,call_id,n) AS sample FROM events WHERE thread_key IS NOT NULL)");
                }
                sql.push_str(" SELECT json_object('trace',trace_id,'call',call_id,'thread',thread,
                    'thread_key',thread_key,'observed',observed,'kind',kind,'detail',json(detail)) ");
                if by_thread {
                    sql.push_str("FROM examples WHERE sample=1");
                } else {
                    sql.push_str("FROM events");
                }
                sql.push_str(" ORDER BY observed DESC,trace_id,call_id,n LIMIT ? OFFSET ?");
            }
            "findings" => {
                sql.push_str(", groups AS (SELECT ");
                let terms = filters.get("terms").map(|text| {
                    text.split_whitespace()
                        .filter(|word| (2..=80).contains(&word.len()))
                        .take(8)
                        .map(str::to_lowercase)
                        .collect::<Vec<_>>()
                });
                if let Some(terms) = terms.as_ref() {
                    // Rank groups by a matching observation while counting every scoped observation.
                    sql.push_str("max((SELECT count(*) FROM json_each(?) term WHERE instr(lower(
                        kind || ' ' || coalesce(json_extract(detail,'$.coverage'),'') || ' ' || coalesce(thread,'') || ' ' || coalesce(json_extract(detail,'$.rule'),'') ||
                        ' ' || coalesce(json_extract(detail,'$.sandbox_dispatch.tool'),'') ||
                        ' ' || coalesce((SELECT group_concat(coalesce(json_extract(s.value,'$.source'),json_extract(s.value,'$.id')),' ')
                            FROM json_each(detail,'$.sources') s),'')),term.value)>0)) AS relevance,");
                    args.push(serde_json::to_string(terms)?);
                }
                sql.push_str("group_id,kind,count(*) AS hits,count(DISTINCT trace_id) AS traces,
                           count(DISTINCT thread_key) AS threads,max(observed) AS latest,
                           CASE
                             WHEN kind='prefill' AND max(json_extract(detail,'$.routing_test') IS NOT NULL) THEN -4
                             WHEN kind='replacement' THEN -4
                             WHEN max(json_extract(detail,'$.joint_pattern')='joint_only') THEN -2
                             WHEN kind='candidate' AND min(json_extract(detail,'$.baseline')*json_extract(detail,'$.edited'))<0 THEN -3
                             WHEN kind='token' OR (kind='shadow' AND max(changed OR proposal_changed)) THEN -2
                             WHEN kind='source_mask' AND max(proposal_changed) THEN -2
                             WHEN max(json_extract(detail,'$.sandbox_dispatch.outcome')='matched_without_check') THEN -2
                             WHEN max(json_extract(detail,'$.sandbox_dispatch.protected'))=1 THEN -2
                             WHEN kind IN ('candidate','prefill') THEN -1
                             WHEN kind IN ('conflict','source_gap','baseline_check') AND count(DISTINCT trace_id)>1 THEN 0
                             WHEN kind='replay' AND max(abs(json_extract(detail,'$.effect_pp')))>0 THEN 1
                             WHEN kind IN ('conflict','source_gap','baseline_check') THEN 2
                             WHEN kind='duplicate' AND min(coalesce(json_extract(detail,'$.dialogue_only'),0))=1 THEN 5
                             WHEN kind='duplicate' THEN 3
                             ELSE 4 END AS priority
                    FROM events GROUP BY group_id,kind ");
                if terms.is_some() {
                    sql.push_str("HAVING relevance > 0 ORDER BY relevance DESC,");
                } else {
                    sql.push_str("ORDER BY ");
                }
                sql.push_str("priority,threads DESC,traces DESC,latest DESC,group_id LIMIT ? OFFSET ?)");
                sql.push_str(super::prefill::SCOPED_COUNTS);
                super::prefill::append_routing(&mut sql, true);
                sql.push_str("                 SELECT json_object('id',g.group_id,'kind',g.kind,'occurrences',g.hits,'traces',g.traces,
                     'threads',g.threads,'latest',g.latest,'priority',g.priority,
                     'joint_patterns',CASE WHEN json_type(e.detail,'$.joint_pattern') IS NOT NULL THEN json((
                         SELECT json_group_array(json_object('pattern',pattern,'conversations',threads,'observations',reports,'inputs',inputs,'missing_inputs',missing_inputs)) FROM (
                             SELECT json_extract(detail,'$.joint_pattern') AS pattern,count(DISTINCT thread_key) AS threads,
                                count(*) AS reports,count(DISTINCT json_extract(detail,'$.joint_input')) AS inputs,
                                count(*) FILTER (WHERE json_extract(detail,'$.joint_input') IS NULL) AS missing_inputs
                             FROM events WHERE group_id=g.group_id AND factor=e.factor
                               AND json_extract(detail,'$.measurement.baseline.attempted')=json_extract(e.detail,'$.measurement.baseline.attempted')
                             GROUP BY pattern ORDER BY pattern))) END,
                     'prefill_followup',CASE WHEN g.kind='prefill' THEN json((SELECT json_group_array(json_object(
                        'runner',runner,'changed',changed,'equal',equal,'mixed',mixed)) FROM
                        (SELECT runner,sum(low=1) AS changed,sum(high=0) AS equal,sum(low!=high) AS mixed
                         FROM prefill_replays WHERE group_id=g.group_id GROUP BY runner ORDER BY runner))) END,
                     'prefill_counts',CASE WHEN g.kind='prefill' THEN json((SELECT json_group_array(json_object('leader',json(leader),'inputs',inputs)) FROM
                       (SELECT leader,inputs FROM prefill_counts WHERE group_id=g.group_id ORDER BY inputs DESC,leader))) END,
                     'routing_design_count',(SELECT count(*) FROM routing_designs d WHERE d.group_id=g.group_id),
                     'routing_profiles',CASE WHEN json_extract(e.detail,'$.routing_test.design') IS NOT NULL THEN json((
                         SELECT json_group_array(json_object('signature',signature,'tool_signature',tool_signature,'inputs',inputs,
                             'example',json_extract(sample,'$[1]'),'repeat_evidence',json_extract(sample,'$[2]'),
                             'example_runs',json_extract(sample,'$[3]'),'repeat_counts',json_extract(sample,'$[4]'))) FROM (
                           SELECT signature,tool_signature,count(*) AS inputs,min(json_array(input,json(example),json(witnesses),report_count,json(counts))) AS sample
                           FROM routing_inputs WHERE group_id=g.group_id AND design=json_extract(e.detail,'$.routing_test.design')
                           GROUP BY signature,tool_signature ORDER BY inputs DESC,signature,tool_signature))) END,
                     'routing_conversations',CASE WHEN json_extract(e.detail,'$.routing_test.design') IS NOT NULL THEN (
                         SELECT count(DISTINCT thread_key) FROM routing_events r
                         WHERE r.group_id=g.group_id AND r.design=json_extract(e.detail,'$.routing_test.design')) END,
                     'mask_outcomes',CASE WHEN g.kind='source_mask' THEN json((SELECT json_group_object(outcome,n) FROM (
                         SELECT outcome,count(*) AS n FROM (
                           SELECT CASE WHEN min(outcome)=max(outcome) THEN min(outcome) ELSE 'mixed' END AS outcome
                           FROM (SELECT trace_id,call_id,json_extract(detail,'$.measurement.location.decision_id') AS decision,
                               json_extract(detail,'$.mask.outcome') AS outcome FROM events WHERE group_id=g.group_id)
                           GROUP BY trace_id,call_id,decision) GROUP BY outcome))) END,
                     'evidence',json_object('producer',e.source,'organization',e.org,'workflow',e.workflow,'thread_key',e.thread_key,'thread',e.thread,'trace_id',e.trace_id,'call_id',e.call_id,'observed',e.observed,'detail',json(e.detail)),
                     'trend',json((SELECT json_group_array(json_object('day',day,'count',hits)) FROM
                       (SELECT date(observed,'unixepoch') AS day,count(*) AS hits FROM events e
                        WHERE e.group_id=g.group_id AND observed IS NOT NULL GROUP BY day ORDER BY day DESC LIMIT 14))))
                 FROM groups g JOIN events e ON (e.trace_id,e.n)=(
                    SELECT chosen.trace_id,chosen.n FROM events chosen
                    LEFT JOIN routing_designs d ON d.group_id=chosen.group_id AND d.design=json_extract(chosen.detail,'$.routing_test.design')
                    WHERE chosen.group_id=g.group_id
                    ORDER BY CASE
                       WHEN chosen.kind='prefill' AND json_extract(chosen.detail,'$.routing_test.pattern') != 'incomplete' THEN -2
                       WHEN json_extract(chosen.detail,'$.joint_pattern')='joint_only' THEN -2
                       WHEN json_extract(chosen.detail,'$.routing_test') IS NOT NULL OR json_extract(chosen.detail,'$.joint_only')=1 THEN -1
                       WHEN chosen.kind='candidate' AND json_extract(chosen.detail,'$.baseline')*json_extract(chosen.detail,'$.edited')<0 THEN 0
                       WHEN json_extract(chosen.detail,'$.sandbox_dispatch.outcome')='matched_without_check' THEN -1
                       WHEN json_extract(chosen.detail,'$.sandbox_dispatch.protected')=1 THEN 0
                       WHEN chosen.kind IN ('shadow','replay','source_mask') AND (chosen.changed OR chosen.proposal_changed) THEN 0 ELSE 1 END,
                       d.completed DESC,d.inputs DESC,chosen.observed DESC,chosen.trace_id,chosen.call_id,chosen.n LIMIT 1)
                 ORDER BY ");
                if terms.is_some() {
                    sql.push_str("g.relevance DESC,");
                }
                sql.push_str("priority,threads DESC,traces DESC,latest DESC,g.group_id");
            }
            "thread-calls" => {
                if super::required(filters, "thread")?.is_empty() {
                    return Err("a conversation key is required".into());
                }
                sql.push_str(super::conversation::START);
                append_window(&mut sql, &mut args, "c.started", filters)?;
                sql.push(')');
                let order = super::conversation::ORDER;
                if filters.contains_key("focus_trace") || filters.contains_key("focus_call") {
                    let mut focus_args = args.clone();
                    focus_args.push(super::required(filters,"focus_trace")?.to_owned());
                    focus_args.push(super::required(filters,"focus_call")?.to_owned());
                    let locate = format!("{sql} SELECT row_index FROM (SELECT trace_id,call_id,row_number() OVER (ORDER BY {order})-1 AS row_index FROM thread_calls) WHERE trace_id=? AND call_id=?");
                    let position: Option<u32> = self.0.query_row(&locate,params_from_iter(focus_args),|r| r.get(0)).optional()?;
                    let Some(position) = position else {
                        return Ok(json!({"rows":[],"offset":0,"next_offset":null}));
                    };
                    offset = position / PAGE as u32 * PAGE as u32;
                }
                sql.push_str(", call_page AS MATERIALIZED (SELECT * FROM thread_calls ORDER BY ");
                sql.push_str(order);
                sql.push_str(super::conversation::PAGE);
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
            "threads" => {
                sql.push_str(" SELECT json_object('id',thread_key,'thread',json_extract(thread_key,'$[3]'),
                      'org',json_extract(thread_key,'$[1]'),'user',json_extract(thread_key,'$[2]'),
                      'start',max(start),'trace_count',count(*)) FROM selected
                      GROUP BY thread_key ORDER BY ");
                if let Some(name) = thread_name {
                    sql.push_str("(lower(json_extract(thread_key,'$[3]'))=lower(?)) DESC,");
                    args.push(name.to_owned());
                }
                sql.push_str("max(start) DESC,thread_key LIMIT ? OFFSET ?");
            }
            _ => return Err("unknown view".into()),
        }
        args.extend([(page_size + 1).to_string(), offset.to_string()]);
        let mut stmt = self.0.prepare(&sql)?;
        let mut rows: Vec<Value> = stmt
            .query_map(params_from_iter(args), |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<_>>()?;
        let next = (rows.len() > page_size).then(|| offset.saturating_add(page_size as u32));
        rows.truncate(page_size);
        if view == "findings" {
            for row in &mut rows {
                if filters
                    .get("group")
                    .is_some_and(|group| row["id"] == group.as_str())
                {
                    if row["kind"] == "conflict" {
                        row["conflict_test"] =
                            super::conflict::followup(&self.0, &row["evidence"])?;
                    } else if row["kind"] == "candidate"
                        && row["evidence"]["detail"]["prefix"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                    {
                        row["prefix_review"] = json!(self.prefix_review(
                            row["evidence"]["trace_id"]
                                .as_str()
                                .ok_or("missing trace")?,
                            row["evidence"]["call_id"].as_str().ok_or("missing call")?,
                            row["evidence"]["detail"]["report_id"]
                                .as_str()
                                .ok_or("missing report")?
                        )?);
                    }
                }
                let kind = row["kind"].as_str().ok_or("finding kind missing")?;
                row["action"] = serde_json::to_value(
                    if row["joint_patterns"]
                        .as_array()
                        .is_some_and(|rows| rows.len() > 1)
                    {
                        recommendations::joint_followup()
                    } else if let Some(test) = row.get("conflict_test") {
                        recommendations::conflict_action(test)
                    } else if kind == "prefill" {
                        recommendations::prefill_action(row)
                    } else {
                        recommendations::action(kind, &row["evidence"]["detail"])
                    },
                )?;
            }
        }
        if view == "replay-variations" {
            for row in &mut rows {
                let matched = super::variation::matched_order(row);
                row["matched_fragment_order"] = json!(matched);
                if matched {
                    let pair = super::position_plan::pair_id(row);
                    let (count,summary): (usize,Option<String>) = self.0.query_row(
                        "SELECT count(*),(SELECT summary FROM position_results WHERE pair=?1 ORDER BY rowid DESC LIMIT 1) FROM position_results WHERE pair=?1",
                        [&pair], |r| Ok((r.get(0)?,r.get(1)?)))?;
                    row["position_pair"] = json!(pair);
                    if let Some(summary) = summary {
                        row["latest_position_result"] = serde_json::from_str(&summary)?;
                        row["position_reports"] = json!(count);
                    }
                }
            }
        }
        if view == "thread-calls" {
            self.restore_tool_previews(&mut rows)?;
            return Ok(json!({"rows":rows,"next_offset":next,"offset":offset}));
        }
        Ok(json!({"rows":rows,"next_offset":next}))
    }

    fn restore_tool_previews(&self, rows: &mut [Value]) -> Result<()> {
        let mut runs = BTreeMap::<String, Option<Run>>::new();
        for row in rows {
            let dialogue = &row["dialogue"];
            // Older exports indexed tool spans as empty previews. Read only
            // candidate spans on the current page to avoid rebuilding the index.
            if !super::conversation::needs_tool_preview(dialogue) {
                continue;
            }
            let trace = row["trace"].as_str().ok_or("missing trace")?;
            let call = row["call"].as_str().ok_or("missing call")?;
            let run = match runs.entry(trace.to_owned()) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(load_run(&self.0, trace)?)
                }
            };
            if let Some(call) = run
                .as_ref()
                .and_then(|run| run.calls.iter().find(|span| span.id == call))
            {
                row["dialogue"] = super::conversation::preview(call);
            }
        }
        Ok(())
    }
}

pub(super) fn append_scope(
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

pub(super) fn append_window(
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

pub(super) fn at(value: Option<&str>) -> Option<i64> {
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
    super::capture::replace(tx, id, run)?;
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
    let workflow: Option<String> = if observations.iter().any(|o| {
        matches!(o.kind, "source_gap" | "baseline_check")
            || (run.organization_id.is_some() && o.detail["sandbox_dispatch"].is_object())
    }) {
        tx.query_row("SELECT workflow FROM traces WHERE id=?", [&run.id], |row| {
            row.get(0)
        })?
    } else {
        None
    };
    let mut insert = tx.prepare("INSERT INTO observations VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
    for (index, mut observation) in observations.into_iter().enumerate() {
        if matches!(observation.kind, "source_gap" | "baseline_check") {
            observation.group = findings::digest(
                json!([observation.group, workflow.as_deref().unwrap_or(&run.id)]).to_string(),
            );
            observation.detail["factor"] = observation.group.clone().into();
        } else {
            observation.detail["factor"] = findings::factor_id(run, &observation).into();
        }
        if observation.detail["sandbox_dispatch"].is_object() {
            if let Some(history) = workflow.as_deref().and_then(|workflow| {
                let call = run
                    .calls
                    .iter()
                    .find(|call| call.id == observation.call_id)?;
                super::history::dispatch_observation(run, call, workflow, &observation.detail)
            }) {
                observation.detail["dispatch_history"] = history;
            }
        }
        let edits = super::outcomes::changed_edits(run, &observation);
        if !edits.is_empty() {
            observation.detail["changed_edits"] = edits.into();
        }
        if let Some(witness) = super::outcomes::held_witness(&observation.detail) {
            observation.detail["held_witness"] = witness;
        }
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
