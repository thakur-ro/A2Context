//! Attach independently generated proposals and reopen their scoped source evidence.
use super::{storage, Rejected, Store, MAX_REPORT_BYTES_PER_TRACE};
use crate::{
    sidecar::{prefix_patterns, required},
    Result,
};
use nexusweave::{otlp, replay};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

impl Store {
    pub fn prefix_check(&self, fields: &BTreeMap<String, String>) -> Result<Option<Value>> {
        let trace = required(fields, "trace")?;
        let call = required(fields, "call")?;
        let report = required(fields, "report")?;
        let _snapshot = self.0.unchecked_transaction()?;
        let page = self.list("prefix-cases", fields)?;
        let Some(example) = page["rows"].get(0) else {
            return Ok(None);
        };
        let Some(review) = self.prefix_review(trace, call, report)? else {
            return Ok(None);
        };
        let mut matching_trial = None;
        if review["verdict"] == "untested" {
            if let Some(key) = example["trial_key"].as_str() {
                let mut scoped = fields.clone();
                for field in [
                    "trace",
                    "call",
                    "report",
                    "verdict",
                    "reference",
                    "ref_set",
                    "ref_outcome",
                    "ref_prediction",
                    "needs_trial",
                    "offset",
                ] {
                    scoped.remove(field);
                }
                scoped.insert("trial_key".into(), key.into());
                scoped.insert("tested".into(), "1".into());
                matching_trial = self.list("prefix-cases", &scoped)?["rows"].get(0).cloned();
            }
        }
        Ok(Some(
            json!({"example":example,"review":review,"matching_trial":matching_trial}),
        ))
    }

    pub fn attach_prefix(
        &self,
        mut scope: BTreeMap<String, String>,
        shadow: replay::shadow::Report,
    ) -> std::result::Result<bool, Rejected> {
        // A retried upload must still resolve after its pending recommendation closes.
        scope.remove("needs_trial");
        scope.remove("verdict");
        scope.remove("ref_outcome");
        let required = |key| required(&scope, key).map_err(|e| (400, e.to_string()));
        let trace_id = required("trace")?;
        let call_id = required("call")?;
        let candidate_id = required("report")?;
        let tx = self.0.unchecked_transaction().map_err(storage)?;
        let witness = self
            .list("prefix-cases", &scope)
            .map_err(|e| (400, e.to_string()))?;
        if witness["rows"].get(0).is_none() {
            return Err((404, "Scored input unavailable in this scope.".into()));
        }
        let run = self
            .trace(trace_id)
            .map_err(storage)?
            .ok_or((404, "Missing trace.".into()))?;
        let call = run
            .calls
            .iter()
            .find(|c| c.id == call_id)
            .ok_or((404, "Missing call.".into()))?;
        let evidence = prefix_patterns::upload_origin(call);
        let trial =
            replay::shadow::attached(trace_id, call, &shadow, &evidence).map_err(|e| (400, e))?;
        let candidates: Vec<_> = replay::candidates::recorded(trace_id, call)
            .into_iter()
            .filter(|c| c.report_id == Some(candidate_id))
            .collect();
        let review = replay::candidates::prefix_reviews(&candidates, &[trial]).pop();
        if !review.is_some_and(|r| {
            matches!(
                r["verdict"].as_str(),
                Some("confirmed" | "disagreed" | "unresolved")
            )
        }) {
            return Err((
                400,
                "The trials must test the selected source edit on this exact request.".into(),
            ));
        }
        let body = serde_json::to_string(&shadow).map_err(storage)?;
        let previous:Option<String> = tx.query_row(
            "SELECT body FROM prefix_trials WHERE trace_id=? AND call_id=? AND runner=? AND report=?",
            params![trace_id,call_id,shadow.runner_id,shadow.id], |r|r.get(0)).optional().map_err(storage)?;
        if let Some(previous) = previous {
            return if serde_json::from_str::<replay::shadow::Report>(&previous).map_err(storage)?
                == shadow
            {
                Ok(false)
            } else {
                Err((
                    409,
                    "This runner and report ID already identify different trials on this call."
                        .into(),
                ))
            };
        }
        if let Some(captured) = prefix_patterns::captured_report(call, &shadow.id)
            .filter(|r| r.runner_id == shadow.runner_id)
        {
            return if captured == &shadow {
                Ok(false)
            } else {
                Err((
                    409,
                    "The captured report has different trials under this runner and report ID."
                        .into(),
                ))
            };
        }
        let (bytes,count):(usize,usize) = tx.query_row("SELECT coalesce(sum(length(CAST(body AS BLOB))),0),count(*) FROM prefix_trials WHERE trace_id=?",
            [trace_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(storage)?;
        if bytes + body.len() > MAX_REPORT_BYTES_PER_TRACE || count >= 128 {
            return Err((
                413,
                "Prefix trial evidence exceeds the per-trace storage limit.".into(),
            ));
        }
        tx.execute(
            "INSERT INTO prefix_trials VALUES(?,?,?,?,?)",
            params![trace_id, call_id, shadow.runner_id, shadow.id, body],
        )
        .map_err(storage)?;
        tx.execute(
            "DELETE FROM prefix_checks WHERE trace_id=? AND call_id=?",
            params![trace_id, call_id],
        )
        .map_err(storage)?;
        prefix_patterns::index(&tx, &run, call).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(true)
    }

    pub fn prefix_input(&self, fields: &BTreeMap<String, String>) -> Result<Option<Value>> {
        let trace_id = required(fields, "trace")?;
        let call_id = required(fields, "call")?;
        let report_id = required(fields, "report")?;
        let _snapshot = self.0.unchecked_transaction()?;
        if self.list("prefix-cases", fields)?["rows"].get(0).is_none() {
            return Ok(None);
        }
        let raw: Option<String> =
            self.0
                .query_row("SELECT raw FROM traces WHERE id=?", [trace_id], |r| {
                    r.get(0)
                })?;
        let Some(raw) = raw else { return Ok(None) };
        let stored: otlp::StoredTrace = serde_json::from_str(&raw)?;
        let Some(request) = stored.pending_request(call_id) else {
            return Ok(None);
        };
        let Some(run) = self.trace(trace_id)? else {
            return Ok(None);
        };
        let Some(candidate) = run
            .calls
            .iter()
            .find(|c| c.id == call_id)
            .and_then(|c| c.candidate_probes.as_ref()?.reports.as_ref())
            .and_then(|reports| reports.iter().find(|r| r.id == report_id))
        else {
            return Ok(None);
        };
        replay::candidates::prefix_trial_case(&request, candidate, "export validation")?;
        Ok(Some(json!({"request":request,"candidate":candidate})))
    }

    pub(super) fn prefix_review(
        &self,
        id: &str,
        call_id: &str,
        report_id: &str,
    ) -> Result<Option<Value>> {
        let Some(run) = self.trace(id)? else {
            return Ok(None);
        };
        let Some(call) = run.calls.iter().find(|c| c.id == call_id) else {
            return Ok(None);
        };
        let candidates = replay::candidates::recorded(id, call);
        let reports: Vec<_> = candidates
            .into_iter()
            .filter(|p| p.report_id == Some(report_id))
            .collect();
        if reports.len() != 1 {
            return Ok(None);
        }
        let uploads = prefix_patterns::uploads(&self.0, id, call_id)?;
        let evidence = prefix_patterns::upload_origin(call);
        let trials = prefix_patterns::trials(id, call, &uploads, &evidence);
        Ok(replay::candidates::prefix_reviews(&reports, &trials).pop())
    }
}
