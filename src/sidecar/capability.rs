//! Locate a stated tool limitation beside a discovery function in the same request.
use super::findings::{digest, occurrence, Observation};
use super::store::{append_scope, append_window, at};
use a2context::{Call, Run};
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS discovery_uses(
    trace_id TEXT NOT NULL REFERENCES traces(id) ON DELETE CASCADE, call_id TEXT NOT NULL,
    model TEXT NOT NULL, catalog TEXT NOT NULL, tool TEXT NOT NULL, started INTEGER,
    invocation TEXT NOT NULL, result TEXT NOT NULL, PRIMARY KEY(trace_id,call_id,tool)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS discovery_catalog ON discovery_uses(catalog,model,tool,started DESC);";

static LIMITATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:(?:the|my) )?(?:(?:available|provided|current) )?tools?(?: (?:available to me|I have))? (?:do not|don't|don’t|cannot|can't|can’t) (?:provide|include|support|allow|offer|access|retrieve)\b|I (?:do not|don't|don’t) have (?:access to )?(?:a |any |the )?(?:tools?|functions?)\b)").unwrap()
});

static DISCOVERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:get|list|show|inspect|describe|enumerate|retrieve|return)s?\b[^.!?\n]{0,96}\b(?:schemas?|catalogs?|tables?|columns?|collections?|datasets?|resources?|metadata|indexes)\b").unwrap()
});

// Frameworks sometimes prefix the docstring with the callable's own signature.
fn discovery_description<'a>(name: &str, description: &'a str) -> Option<&'a str> {
    let text = description.trim_start();
    if DISCOVERY.is_match(text) {
        return Some(text);
    }
    let (signature, docstring) = text.split_once('\n')?;
    let result = signature
        .trim_end()
        .strip_prefix(name)?
        .strip_prefix("()")?;
    if !result.is_empty() {
        let annotation = result.strip_prefix(" -> ")?;
        if annotation.is_empty()
            || !annotation
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.,[] |".contains(c))
        {
            return None;
        }
    }
    let docstring = docstring.trim_start();
    DISCOVERY.is_match(docstring).then_some(docstring)
}

fn discovery(tool: &Value) -> Option<(&Value, &str)> {
    let f = &tool["function"];
    let params = &f["parameters"];
    let description = discovery_description(f["name"].as_str()?, f["description"].as_str()?)?;
    (tool["type"] == "function"
        && !f["name"].as_str()?.is_empty()
        && params["type"] == "object"
        && params["properties"].as_object()?.is_empty()
        && params
            .get("required")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)))
    .then_some((f, description))
}

pub(super) fn finding(run: &Run, call: &Call) -> Option<Observation> {
    let [reply] = call.decisions.as_slice() else {
        return None;
    };
    if reply.kind != "model_output" {
        return None;
    }
    let text = reply.outcome.as_str()?.trim_start();
    if !LIMITATION.is_match(text) {
        return None;
    }
    let request: Value = serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
    if request.get("tool_choice").is_some_and(|v| v != "auto")
        || request.get("n").is_some_and(|v| v.as_u64() != Some(1))
    {
        return None;
    }
    let messages = request["messages"].as_array()?;
    if messages.last()?["role"] != "user"
        || !messages
            .iter()
            .all(|m| matches!(m["role"].as_str(), Some("user" | "system" | "developer")))
    {
        return None;
    }
    let catalog = request["tools"].as_array()?;
    let (index, (function, description)) = catalog
        .iter()
        .enumerate()
        .find_map(|(i, t)| Some((i, discovery(t)?)))?;
    let catalog_key = digest(request["tools"].to_string());
    let excerpt = |s: &str| s.chars().take(256).collect::<String>();
    Some(occurrence(
        "capability_claim",
        json!([
            "capability-claim-v1",
            run.source,
            run.organization_id,
            call.model,
            catalog_key
        ]),
        call,
        json!({"tool":function["name"],"description":excerpt(description.trim()),
            "catalog":catalog_key, "model":call.model, "pointer":format!("/tools/{index}/function"), "reply":excerpt(text), "decision":reply.id,
            "reply_evidence":reply.evidence, "question_pointer":format!("/messages/{}",messages.len()-1),
            "question": messages.last()?["content"].as_str().map(excerpt), "sources":[]}),
    ))
}

// Only an invocation followed by a tool-role result with the same unique ID
// supplies a comparison. Do not store result content in the lookup table.
pub(super) fn index(tx: &Transaction<'_>, run: &Run) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM discovery_uses WHERE trace_id=?", [&run.id])?;
    let mut insert = tx.prepare("INSERT OR IGNORE INTO discovery_uses VALUES(?,?,?,?,?,?,?,?)")?;
    for call in &run.calls {
        let (Some(model), Some(raw)) = (call.model.as_deref(), call.recorded_request.as_ref())
        else {
            continue;
        };
        let Ok(body) = serde_json::from_str::<Value>(&raw.body) else {
            continue;
        };
        let (Some(tools), Some(messages)) = (body["tools"].as_array(), body["messages"].as_array())
        else {
            continue;
        };
        if !messages.iter().any(|m| m["role"] == "tool") {
            continue;
        }
        let names: BTreeSet<_> = tools
            .iter()
            .filter_map(discovery)
            .filter_map(|(f, _)| f["name"].as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        let mut prior = HashMap::new();
        let key = digest(body["tools"].to_string());
        for (i, m) in messages.iter().enumerate() {
            if m["role"] == "assistant" {
                for (j, t) in m["tool_calls"].as_array().into_iter().flatten().enumerate() {
                    if let Some(id) = t["id"].as_str().filter(|s| !s.is_empty()) {
                        prior
                            .entry(id)
                            .and_modify(|entry| *entry = None)
                            .or_insert(t["function"]["name"].as_str().map(|name| (name, i, j)));
                    }
                }
            } else if m["role"] == "tool" && !m["content"].is_null() {
                if let Some(Some((name, k, j))) =
                    m["tool_call_id"].as_str().and_then(|id| prior.get(id))
                {
                    if names.contains(name) {
                        insert.execute(params![
                            run.id,
                            call.id,
                            model,
                            key,
                            name,
                            at(call.started_at.as_deref()),
                            format!("/messages/{k}/tool_calls/{j}"),
                            format!("/messages/{i}")
                        ])?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn comparison(
    db: &Connection,
    card: &Value,
    filters: &BTreeMap<String, String>,
) -> crate::Result<Option<Value>> {
    let evidence = &card["evidence"];
    let detail = &evidence["detail"];
    let (Some(trace), Some(model), Some(catalog), Some(tool)) = (
        evidence["trace_id"].as_str(),
        detail["model"].as_str(),
        detail["catalog"].as_str(),
        detail["tool"].as_str(),
    ) else {
        return Ok(None);
    };
    let mut sql = "SELECT u.trace_id,u.call_id,u.invocation,u.result,t.thread FROM discovery_uses u
        JOIN traces t ON t.id=u.trace_id JOIN traces origin ON origin.id=? WHERE u.catalog=? AND u.model=? AND u.tool=?
        AND t.id!=origin.id AND t.workflow=origin.workflow AND t.org IS origin.org AND t.user IS origin.user
        AND t.source=origin.source".to_string();
    let mut args = vec![trace.into(), catalog.into(), model.into(), tool.into()];
    append_scope(&mut sql, &mut args, filters)?;
    append_window(&mut sql, &mut args, "u.started", filters)?;
    sql.push_str(" ORDER BY u.started DESC,u.trace_id,u.call_id LIMIT 1");
    Ok(db.query_row(&sql,params_from_iter(args),|r| Ok(json!({"trace":r.get::<_,String>(0)?,
        "call":r.get::<_,String>(1)?,"invocation":r.get::<_,String>(2)?,"result":r.get::<_,String>(3)?,
        "thread":r.get::<_,Option<String>>(4)?}))).optional()?)
}
