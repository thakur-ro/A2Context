//! Pair tool-role messages with prior assistant tool calls in a captured chat request.
use super::store::{append_scope, append_window};
use crate::Result;
use a2context::Run;
use rusqlite::{params, params_from_iter, Connection, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tool_reference_scores (
    trace_id TEXT PRIMARY KEY REFERENCES traces(id) ON DELETE CASCADE,
    status TEXT NOT NULL, call_id TEXT, message_index INTEGER
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS tool_reference_status ON tool_reference_scores(status,trace_id);";

#[derive(PartialEq, Debug)]
enum Check {
    Outside,
    Unknown,
    Unclassified,
    Clear,
    Affected(usize),
}

fn check(body: &Value) -> Check {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return Check::Unclassified;
    };
    let mut counts = HashMap::new();
    let mut saw_result = false;
    let mut incomplete = false;
    for (index, message) in messages.iter().enumerate() {
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(calls) = message.get("tool_calls").filter(|v| !v.is_null()) {
                    if let Some(entries) = calls.as_array() {
                        for entry in entries {
                            if let Some(id) = entry["id"].as_str().filter(|s| !s.is_empty()) {
                                *counts.entry(id).or_insert(0u32) += 1;
                            } else {
                                incomplete = true;
                            }
                        }
                    } else {
                        incomplete = true;
                    }
                }
            }
            Some("tool") => {
                saw_result = true;
                if message.get("content").is_none() {
                    incomplete = true;
                    continue;
                }
                let Some(id) = message["tool_call_id"].as_str().filter(|s| !s.is_empty()) else {
                    return Check::Affected(index);
                };
                let count = counts.get(id).copied().unwrap_or(0);
                if count > 1 || (count == 0 && !incomplete) {
                    return Check::Affected(index);
                }
            }
            Some("user" | "developer" | "system") => {}
            _ => incomplete = true,
        }
    }
    match (saw_result, incomplete) {
        (true, false) => Check::Clear,
        (true, true) => Check::Unknown,
        (false, true) => Check::Unclassified,
        (false, false) => Check::Outside,
    }
}

pub(super) fn index(
    tx: &Transaction<'_>,
    run: &Run,
    inference_calls: usize,
) -> rusqlite::Result<bool> {
    let mut complete = 0;
    let mut eligible = false;
    let mut uncertain = false;
    let mut issue = None;
    for call in &run.calls {
        let input_result = call
            .context
            .iter()
            .any(|item| item.role.as_deref() == Some("tool"));
        let Some(capture) = &call.recorded_request else {
            if input_result {
                eligible = true;
                uncertain = true;
            }
            continue;
        };
        let check =
            serde_json::from_str(&capture.body).map_or(Check::Unclassified, |body| check(&body));
        match check {
            Check::Affected(index) => {
                issue = Some((&call.id, index));
                break;
            }
            Check::Outside => complete += 1,
            Check::Clear => {
                complete += 1;
                eligible = true;
            }
            Check::Unknown | Check::Unclassified => {
                eligible |= input_result || check == Check::Unknown;
                uncertain = true;
            }
        }
    }
    let status = if issue.is_some() {
        "affected"
    } else if uncertain || complete < inference_calls {
        if eligible {
            "unknown"
        } else {
            "unknown_applicability"
        }
    } else if eligible {
        "clear"
    } else {
        "outside"
    };
    tx.execute(
        "INSERT OR REPLACE INTO tool_reference_scores VALUES (?1,?2,?3,?4)",
        params![run.id, status, issue.map(|v| v.0), issue.map(|v| v.1)],
    )?;
    Ok(issue.is_some())
}

pub(super) fn backfill(db: &mut Connection) -> Result<()> {
    let mut after = String::new();
    loop {
        let tx = db.transaction()?;
        let mut select = tx.prepare("SELECT t.id,t.calls,t.run FROM traces t WHERE t.id>?1 AND t.run IS NOT NULL
            AND NOT EXISTS(SELECT 1 FROM tool_reference_scores s WHERE s.trace_id=t.id) ORDER BY t.id LIMIT 16")?;
        let mut rows = select.query([&after])?;
        let mut changed = false;
        while let Some(row) = rows.next()? {
            after = row.get(0)?;
            let run: Run = serde_json::from_slice(row.get_ref(2)?.as_str()?.as_bytes())?;
            if index(&tx, &run, row.get(1)?)? {
                super::store::refresh_findings(&tx, &run)?;
            }
            changed = true;
        }
        drop(rows);
        drop(select);
        tx.commit()?;
        if !changed {
            return Ok(());
        }
    }
}

pub(super) fn witness(call: &a2context::Call) -> Option<Value> {
    let capture = call.recorded_request.as_ref()?;
    let body: Value = serde_json::from_str(&capture.body).ok()?;
    let Check::Affected(index) = check(&body) else {
        return None;
    };
    let messages = body["messages"].as_array()?;
    let id = messages[index]["tool_call_id"]
        .as_str()
        .filter(|s| !s.is_empty());
    let matches = id.map(|id| {
        messages[..index]
            .iter()
            .filter(|m| m["role"] == "assistant")
            .filter_map(|m| m["tool_calls"].as_array())
            .flatten()
            .filter(|c| c["id"].as_str() == Some(id))
            .count()
    });
    Some(
        json!({"pointer":format!("/messages/{index}"),"has_invocation_id":id.is_some(),"matching_prior_calls":matches}),
    )
}

pub(super) fn score(db: &Connection, filters: &BTreeMap<String, String>) -> Result<Value> {
    let snapshot = db.unchecked_transaction()?;
    let mut from =
        " FROM tool_reference_scores s JOIN traces t ON t.id=s.trace_id WHERE 1".to_owned();
    let mut args = vec![];
    append_scope(&mut from, &mut args, filters)?;
    append_window(&mut from, &mut args, "t.start", filters)?;
    let totals = format!("SELECT count(*) FILTER(WHERE status='clear'),count(*) FILTER(WHERE status='affected'),
        count(*) FILTER(WHERE status='unknown'),count(*) FILTER(WHERE status='unknown_applicability'),
        count(*) FILTER(WHERE status='outside'){from}");
    let (c, b, u, a, o): (u64, u64, u64, u64, u64) =
        snapshot.query_row(&totals, params_from_iter(&args), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
    let mut stmt = snapshot.prepare(&format!("SELECT trace_id,call_id,message_index{from} AND status='affected' ORDER BY t.start DESC,t.id LIMIT 5"))?;
    let examples = stmt
        .query_map(params_from_iter(&args), |row| {
            Ok(json!({
                "trace":row.get::<_,String>(0)?, "call":row.get::<_,String>(1)?,
                "pointer":format!("/messages/{}",row.get::<_,usize>(2)?)
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let percent = |n, d| (d != 0).then(|| 100.0 * n as f64 / d as f64);
    Ok(
        json!({"check":"chat-tool-result-reference-v1","unit":"recorded trace","clear":c,"affected":b,
        "unresolved":u,"unknown_applicability":a,"outside":o,"pass_pct":percent(c,c+b),
        "coverage_pct":percent(c+b,c+b+u),"lower_pct":percent(c,c+b+u),"upper_pct":percent(c+u,c+b+u),"examples":examples}),
    )
}
