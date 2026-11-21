//! Compare answer references with input lines shared by adjacent captures in the same scope.
use crate::Result;
use a2context::{Call, Run};
use regex::Regex;
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::findings::digest;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS identifier_inputs (
    trace TEXT PRIMARY KEY REFERENCES traces(id) ON DELETE CASCADE, scope TEXT NOT NULL,
    start_ns INTEGER NOT NULL, end_ns INTEGER NOT NULL, call_index INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS identifier_neighbors ON identifier_inputs(scope,start_ns,trace);
CREATE INDEX IF NOT EXISTS identifier_witness ON observations(json_extract(detail,'$.peer.trace'))
    WHERE kind='shared_identifier';
CREATE TRIGGER IF NOT EXISTS expire_identifier_witness AFTER DELETE ON identifier_inputs BEGIN
    DELETE FROM observations WHERE kind='shared_identifier' AND json_extract(detail,'$.peer.trace')=OLD.trace;
END;";

static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)+").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://[^\s<>]+").unwrap());
static REFERENCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^[ \t]*(?:sources?|references?|citations?)[ \t]*:[^\r\n]*|\([^()\r\n]{1,256}\)",
    )
    .unwrap()
});

#[derive(Serialize)]
struct Match<'a> {
    key: &'a str,
    input_excerpt: String,
    output_bytes: [usize; 2],
    message: usize,
    input_bytes: [usize; 2],
    peer_bytes: [usize; 2],
}

fn shared_bounds(text: &str, prior: &str) -> (usize, usize) {
    let front = text
        .split_inclusive('\n')
        .zip(prior.split_inclusive('\n'))
        .take_while(|(a, b)| a == b)
        .map(|(s, _)| s.len())
        .sum::<usize>();
    let suffix = text[front..]
        .split_inclusive('\n')
        .rev()
        .zip(prior[front..].split_inclusive('\n').rev())
        .take_while(|(a, b)| a == b)
        .map(|(s, _)| s.len())
        .sum::<usize>();
    (front, text.len() - suffix)
}

fn tokens(text: &str) -> impl Iterator<Item = regex::Match<'_>> {
    TOKEN.find_iter(text).filter(|m| {
        let boundary = |c: char| !(c.is_alphanumeric() || c == '_' || c == '-');
        m.len() <= 128
            && m.as_str().bytes().any(|c| c.is_ascii_digit())
            && m.as_str().bytes().any(|c| c.is_ascii_alphabetic())
            && text[..m.start()].chars().next_back().is_none_or(boundary)
            && text[m.end()..].chars().next().is_none_or(boundary)
    })
}

fn line_excerpt(text: &str, [start, end]: [usize; 2]) -> String {
    let before = text[..start]
        .chars()
        .rev()
        .take(64)
        .take_while(|c| *c != '\n')
        .collect::<Vec<_>>();
    let after = text[end..].chars().take(64).take_while(|c| *c != '\n');
    before
        .into_iter()
        .rev()
        .chain(text[start..end].chars())
        .chain(after)
        .collect()
}

fn matches<'a>(texts: &[&str], prior: &[&str], answer: &'a str) -> Vec<Match<'a>> {
    if texts == prior {
        return Vec::new();
    }
    let mut requested = BTreeMap::new();
    for cited in REFERENCES.find_iter(answer) {
        for m in tokens(cited.as_str()) {
            requested
                .entry(m.as_str())
                .or_insert([cited.start() + m.start(), cited.start() + m.end()]);
            if requested.len() > 128 {
                return Vec::new();
            }
        }
    }
    if requested.is_empty() {
        return Vec::new();
    }
    let mut excluded = BTreeSet::new();
    let mut locations = BTreeMap::new();
    for (i, (&text, &old)) in texts.iter().zip(prior).enumerate() {
        let (front, back) = shared_bounds(text, old);
        for m in tokens(text) {
            let key = m.as_str();
            if !requested.contains_key(key) {
                continue;
            }
            let peer = if m.end() <= front {
                m.start()
            } else if m.start() >= back {
                old.len() - (text.len() - m.start())
            } else {
                excluded.insert(key);
                continue;
            };
            locations
                .entry(key)
                .or_insert((i, [m.start(), m.end()], [peer, peer + m.len()]));
        }
    }
    for text in texts.iter().copied().chain([answer]) {
        for url in URL.find_iter(text) {
            excluded.extend(tokens(url.as_str()).map(|m| m.as_str()));
        }
    }
    let mut result: Vec<_> = requested
        .into_iter()
        .filter_map(|(key, output_bytes)| {
            if excluded.contains(key) {
                return None;
            }
            let &(message, input_bytes, peer_bytes) = locations.get(key)?;
            Some(Match {
                key,
                input_excerpt: line_excerpt(texts[message], input_bytes),
                output_bytes,
                message,
                input_bytes,
                peer_bytes,
            })
        })
        .collect();
    result.sort_by_key(|m| m.output_bytes);
    result.truncate(3);
    result
}

fn messages(body: &Value) -> Option<Vec<&str>> {
    if [
        "tools",
        "functions",
        "attachments",
        "previous_response_id",
        "web_search_options",
        "modalities",
    ]
    .iter()
    .any(|k| {
        body.get(*k)
            .is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
    }) {
        return None;
    }
    body["messages"]
        .as_array()?
        .iter()
        .map(|m| {
            if !matches!(
                m["role"].as_str(),
                Some("system" | "developer" | "user" | "assistant")
            ) || m.get("tool_calls").is_some()
                || m.get("function_call").is_some()
            {
                return None;
            }
            m["content"].as_str()
        })
        .collect()
}

fn nanos(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()?
        .unix_timestamp_nanos()
        .try_into()
        .ok()
}

struct Slot {
    scope: String,
    started: i64,
    ended: i64,
    index: usize,
}

fn slot(run: &Run, meta: &str, workflow: &str) -> Option<Slot> {
    run.conversation_id.as_deref().filter(|s| !s.is_empty())?;
    let (started, index, call) = run
        .calls
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((nanos(c.started_at.as_deref()?)?, i, c)))
        .max_by(|a, b| (a.0, &a.2.id).cmp(&(b.0, &b.2.id)))?;
    let [decision] = call.decisions.as_slice() else {
        return None;
    };
    if decision.kind != "model_output" || decision.outcome.as_str()?.len() > 32768 {
        return None;
    }
    let raw = &call.recorded_request.as_ref()?.body;
    if raw.len() > 131072 {
        return None;
    }
    let body: Value = serde_json::from_str(raw).ok()?;
    messages(&body)?;
    let roles: Vec<_> = body["messages"]
        .as_array()?
        .iter()
        .map(|m| &m["role"])
        .collect();
    let scope = digest(
        json!([
            run.source,
            run.organization_id,
            run.user_id,
            workflow,
            meta,
            call.model.as_ref()?,
            roles
        ])
        .to_string(),
    );
    let ended = nanos(call.ended_at.as_deref()?)?;
    (ended >= started).then_some(Slot {
        scope,
        started,
        ended,
        index,
    })
}

fn successor(tx: &Transaction<'_>, trace: &str) -> Result<Option<String>> {
    Ok(tx.query_row("SELECT b.trace FROM identifier_inputs a JOIN identifier_inputs b ON b.scope=a.scope
        AND (b.start_ns,b.trace)>(a.start_ns,a.trace) WHERE a.trace=? ORDER BY b.start_ns,b.trace LIMIT 1",
        [trace],|r|r.get(0)).optional()?)
}

pub(super) fn update(
    tx: &Transaction<'_>,
    id: &str,
    run: Option<&Run>,
    meta: &str,
    workflow: Option<&str>,
) -> Result<()> {
    let mut affected = BTreeSet::new();
    affected.extend(successor(tx, id)?);
    tx.execute("DELETE FROM identifier_inputs WHERE trace=?", [id])?;
    tx.execute(
        "DELETE FROM observations WHERE trace_id=? AND kind='shared_identifier'",
        [id],
    )?;
    if let Some(s) = run.zip(workflow).and_then(|(r, w)| slot(r, meta, w)) {
        tx.execute(
            "INSERT INTO identifier_inputs VALUES(?,?,?,?,?)",
            params![id, s.scope, s.started, s.ended, s.index],
        )?;
        affected.insert(id.to_owned());
        affected.extend(successor(tx, id)?);
    }
    for target in affected {
        compare(tx, &target)?;
    }
    Ok(())
}

fn compare(tx: &Transaction<'_>, trace: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM observations WHERE trace_id=? AND kind='shared_identifier'",
        [trace],
    )?;
    // Pick the immediate predecessor before inspecting either payload. Time ties, overlapping
    // calls and same-thread pairs abstain. The index bounds work even for long-lived workflows.
    let mut stmt=tx.prepare_cached("WITH pair AS (SELECT a.scope,a.start_ns,a.call_index ci,b.call_index pi,b.end_ns,b.trace peer
        FROM identifier_inputs a JOIN identifier_inputs b ON b.scope=a.scope AND (b.start_ns,b.trace)<(a.start_ns,a.trace)
        WHERE a.trace=?1 ORDER BY b.start_ns DESC,b.trace DESC LIMIT 1)
        SELECT pair.scope,a.thread,b.id,b.thread,json_extract(a.run,'$.calls['||ci||']'),
               json_extract(b.run,'$.calls['||pi||'].recorded_request.body'),json_extract(b.run,'$.calls['||pi||'].id')
        FROM pair JOIN traces a ON a.id=?1 JOIN traces b ON b.id=peer WHERE end_ns<start_ns AND a.thread!=b.thread")?;
    let mut rows = stmt.query([trace])?;
    let Some(row) = rows.next()? else {
        return Ok(());
    };
    let scope: &str = row.get_ref(0)?.as_str()?;
    let call: Call = serde_json::from_str(row.get_ref(4)?.as_str()?)?;
    let raw: Value = serde_json::from_str(
        &call
            .recorded_request
            .as_ref()
            .ok_or("missing indexed request")?
            .body,
    )?;
    let prior: Value = serde_json::from_str(row.get_ref(5)?.as_str()?)?;
    let [reply] = call.decisions.as_slice() else {
        return Ok(());
    };
    let Some((texts, peer)) = messages(&raw).zip(messages(&prior)) else {
        return Ok(());
    };
    let found = matches(&texts, &peer, reply.outcome.as_str().unwrap_or_default());
    if found.is_empty() {
        return Ok(());
    }
    let messages: BTreeSet<_> = found.iter().map(|m| m.message).collect();
    let group = digest(json!(["shared-identifier-v1", scope, messages]).to_string());
    let detail = json!({"factor":group,"matches":found,"decision":reply.id,"sources":[],
        "peer":{"trace":row.get_ref(2)?.as_str()?,"thread":row.get_ref(3)?.as_str()?,"call":row.get_ref(6)?.as_str()?}});
    tx.execute(
        "INSERT INTO observations VALUES(?,-1,?,'shared_identifier',?,?,?)",
        params![
            trace,
            call.id,
            group,
            super::store::at(call.started_at.as_deref()),
            detail.to_string()
        ],
    )?;
    Ok(())
}

pub(super) fn purge(tx: &Transaction<'_>, cutoff: i64) -> Result<usize> {
    // Only the first survivor after a removed sequence can gain a new comparison.
    let mut query = tx.prepare_cached("SELECT DISTINCT next FROM (
        SELECT (SELECT b.trace FROM identifier_inputs b WHERE b.scope=a.scope
                AND (b.start_ns,b.trace)>(a.start_ns,a.trace) ORDER BY b.start_ns,b.trace LIMIT 1) AS next
        FROM traces t JOIN identifier_inputs a ON a.trace=t.id WHERE t.seen<?1)
        JOIN traces survivor ON survivor.id=next AND survivor.seen>=?1")?;
    let successors = query
        .query_map([cutoff], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let removed = tx.execute("DELETE FROM traces WHERE seen < ?1", [cutoff])?;
    for target in successors {
        compare(tx, &target)?;
    }
    Ok(removed)
}
