//! Search the same bounded dialogue that the dashboard can show.
use rusqlite::{params, params_from_iter, Connection, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::store::{append_scope, append_window};
use crate::Result;

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS activity_documents (
        id INTEGER PRIMARY KEY, trace_id TEXT NOT NULL, call_id TEXT NOT NULL,
        UNIQUE(trace_id,call_id), FOREIGN KEY(trace_id,call_id) REFERENCES conversation_calls ON DELETE CASCADE
    );
    CREATE VIRTUAL TABLE IF NOT EXISTS activity_terms USING fts5(
        message, result, content='', contentless_delete=1, tokenize='porter unicode61'
    );
    CREATE TRIGGER IF NOT EXISTS activity_document_delete AFTER DELETE ON activity_documents BEGIN
        DELETE FROM activity_terms WHERE rowid=old.id;
    END;
    CREATE TABLE IF NOT EXISTS activity_index_progress (id INTEGER PRIMARY KEY CHECK(id=1), cursor TEXT);
    INSERT OR IGNORE INTO activity_index_progress VALUES (1,'');";

pub(super) fn index(
    tx: &Transaction<'_>,
    trace: &str,
    call: &str,
    preview: &Value,
) -> rusqlite::Result<()> {
    let message = preview["user"]["text"].as_str().unwrap_or_default();
    let result = preview["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|output| [output["label"].as_str(), output["text"].as_str()])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    if message.is_empty() && result.is_empty() {
        tx.prepare_cached("DELETE FROM activity_documents WHERE trace_id=? AND call_id=?")?
            .execute(params![trace, call])?;
        return Ok(());
    }
    tx.prepare_cached("INSERT OR IGNORE INTO activity_documents(trace_id,call_id) VALUES (?1,?2)")?
        .execute(params![trace, call])?;
    tx.prepare_cached(
        "INSERT OR REPLACE INTO activity_terms(rowid,message,result)
        SELECT id,?3,?4 FROM activity_documents WHERE trace_id=?1 AND call_id=?2",
    )?
    .execute(params![trace, call, message, result])?;
    Ok(())
}

pub(super) fn cursor(conn: &Connection) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT cursor FROM activity_index_progress WHERE id=1",
        [],
        |row| row.get(0),
    )
}

/// A small sample of recorded tool names gives the question planner the local vocabulary.
pub(super) fn tool_names(db: &Connection, filters: &BTreeMap<String, String>) -> Result<Value> {
    let mut sql = "WITH recent AS (SELECT p.preview FROM captured_calls c
        JOIN traces t ON t.id=c.trace_id
        JOIN conversation_calls p ON (p.trace_id,p.call_id)=(c.trace_id,c.call_id) WHERE 1"
        .to_owned();
    let mut args = Vec::new();
    append_scope(&mut sql, &mut args, filters)?;
    append_window(&mut sql, &mut args, "c.started", filters)?;
    sql.push_str(" ORDER BY c.started DESC,c.trace_id DESC,c.call_id DESC LIMIT 64)
        SELECT j.value->>'label' AS name FROM recent,json_each(recent.preview,'$.outputs') j
        WHERE j.value->>'kind' IN ('tool_call','tool_execution') AND length(j.value->>'label') BETWEEN 1 AND 100
        GROUP BY name ORDER BY count(*) DESC,name LIMIT 16");
    let mut stmt = db.prepare(&sql)?;
    let names = stmt
        .query_map(params_from_iter(args), |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"tools":names}))
}

pub(super) fn search(db: &Connection, filters: &BTreeMap<String, String>) -> Result<Value> {
    let query = filters.get("q").map(String::as_str).unwrap_or_default();
    if query.chars().count() > 240 {
        return Err("activity search is limited to 240 characters".into());
    }
    let mut seen = BTreeSet::new();
    let terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty() && seen.insert(term.to_lowercase()))
        .take(12)
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>();
    let ready = cursor(db)?.is_none();
    if terms.is_empty() {
        return Ok(json!({"rows":[],"index_ready":ready}));
    }
    let mut sql = "SELECT json_object('trace',t.id,'call',d.call_id,'thread',t.thread,'thread_id',t.thread_key,
        'workflow',t.workflow,'dialogue',json(p.preview)) FROM activity_terms
        JOIN activity_documents d ON d.id=activity_terms.rowid JOIN traces t ON t.id=d.trace_id
        JOIN captured_calls c ON (c.trace_id,c.call_id)=(d.trace_id,d.call_id)
        JOIN conversation_calls p ON (p.trace_id,p.call_id)=(d.trace_id,d.call_id)
        WHERE activity_terms MATCH ?".to_owned();
    let mut args = vec![terms.join(" AND ")];
    append_scope(&mut sql, &mut args, filters)?;
    append_window(&mut sql, &mut args, "c.started", filters)?;
    if let Some(tool) = filters.get("tool") {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM json_each(p.preview,'$.outputs') j
            WHERE j.value->>'kind' IN ('tool_call','tool_execution') AND j.value->>'label'=?)",
        );
        args.push(tool.clone());
    }
    sql.push_str(" ORDER BY activity_terms.rank,c.started DESC,t.id,d.call_id LIMIT 48");
    let mut stmt = db.prepare(&sql)?;
    let mut read = |args: &[String]| -> Result<Vec<Value>> {
        let mut groups = BTreeSet::new();
        let mut matches = Vec::new();
        for row in stmt.query_map(params_from_iter(args), |row| row.get::<_, String>(0))? {
            let row: Value = serde_json::from_str(&row?)?;
            if let Some(key) = row["thread_id"].as_str().or(row["trace"].as_str()) {
                if groups.insert(key.to_owned()) {
                    matches.push(row);
                }
            }
            if matches.len() == 4 {
                break;
            }
        }
        Ok(matches)
    };
    let mut rows = read(&args)?;
    let partial = rows.is_empty()
        && terms.len() > 1
        && !filters.get("match").is_some_and(|mode| mode == "all");
    let mut matched_terms = terms.len();
    if partial && terms.len() > 2 {
        // Prefer a shared pair over an isolated word when the full query misses.
        args[0] = terms
            .iter()
            .enumerate()
            .flat_map(|(i, left)| {
                terms[i + 1..]
                    .iter()
                    .map(move |right| format!("({left} AND {right})"))
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        rows = read(&args)?;
        matched_terms = 2;
    }
    if partial && rows.is_empty() {
        args[0] = terms.join(" OR ");
        rows = read(&args)?;
        matched_terms = 1;
    }
    Ok(
        json!({"rows":rows,"index_ready":ready,"partial_match":partial,
        "minimum_matching_terms":if rows.is_empty() { 0 } else { matched_terms }}),
    )
}
