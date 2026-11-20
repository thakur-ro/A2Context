//! Resolve the evidence and editable fields for one declared disagreement on one call.
use crate::Result;
use nexusweave::{replay, Availability, Call, Decision, Run};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn eligible(decision: &Decision, detail: &Value) -> bool {
    matches!(decision.kind.as_str(), "model_output" | "tool_call")
        && !decision.outcome.is_null()
        && decision
            .applies_to
            .as_ref()
            .is_none_or(|s| detail["rule"] == s.key && detail["scope"] == s.scope)
}

/// Keep each report's two individual arms together. Separate reports may target
/// different decisions, models or replacements even on the same call.
pub(super) fn annotate(
    run: &Run,
    reports: &[replay::Replay],
    findings: &mut [super::findings::Observation],
) {
    if reports.is_empty() || !findings.iter().any(|f| f.kind == "conflict") {
        return;
    }
    let mut measured = BTreeMap::new();
    for finding in findings.iter().filter(|f| {
        f.kind == "replay"
            && f.detail["edit"] == "individual"
            && f.detail["response_pair"]["changed"].is_boolean()
    }) {
        let m = &finding.detail["measurement"];
        if let (Some(decision), Some(report), Some(source)) = (
            m["location"]["decision_id"].as_str(),
            m["replay_id"].as_str(),
            finding.detail["sources"][0]["id"].as_str(),
        ) {
            measured
                .entry(finding.call_id.clone())
                .or_insert_with(BTreeMap::new)
                .entry((decision.to_owned(), report.to_owned()))
                .or_insert_with(BTreeSet::new)
                .insert(source.to_owned());
        }
    }
    if measured.is_empty() {
        return;
    }
    for finding in findings.iter_mut().filter(|f| f.kind == "conflict") {
        let Some(trials) = measured.get(&finding.call_id) else {
            continue;
        };
        let Some(call) = run.calls.iter().find(|c| c.id == finding.call_id) else {
            continue;
        };
        let sources = &finding.detail["sources"];
        let mut individual = BTreeSet::new();
        let mut together = false;
        for ((decision, _), edits) in trials {
            if !call
                .decisions
                .iter()
                .any(|d| d.id == *decision && eligible(d, &finding.detail))
            {
                continue;
            }
            let matched: Vec<_> = sources
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["id"].as_str())
                .filter(|id| edits.contains(*id))
                .collect();
            together |= matched.len() == 2;
            individual.extend(matched);
        }
        if !individual.is_empty() {
            finding.detail["conflict_trials"] =
                json!({"sources":individual,"same_report":together});
        }
    }
}

pub(super) fn followup(db: &Connection, evidence: &Value) -> Result<Value> {
    let trace = evidence["trace_id"]
        .as_str()
        .ok_or("missing conflict trace")?;
    let call_id = evidence["call_id"]
        .as_str()
        .ok_or("missing conflict call")?;
    let detail = &evidence["detail"];
    let raw: String = db.query_row(
        "SELECT c.value FROM traces t,json_each(t.run,'$.calls') c
        WHERE t.id=? AND json_extract(c.value,'$.id')=? LIMIT 1",
        [trace, call_id],
        |r| r.get(0),
    )?;
    let call: Call = serde_json::from_str(&raw)?;
    let body = call
        .recorded_request
        .as_ref()
        .map(|r| serde_json::from_str(&r.body))
        .transpose()?;
    let fields = body
        .as_ref()
        .map(|r| replay::request_field_matches(&call, r))
        .transpose()?
        .unwrap_or_default();
    let decisions: Vec<_> = call
        .decisions
        .iter()
        .filter(|d| eligible(d, detail))
        .collect();
    let ids = serde_json::to_string(&decisions.iter().map(|d| &d.id).collect::<Vec<_>>())?;
    let mut query = db.prepare_cached(
        "WITH trials AS (SELECT group_id,n,json_extract(detail,'$.response_pair.changed') AS changed,
         json_extract(detail,'$.measurement.replay_id') AS replay,json_extract(detail,'$.measurement.location.decision_id') AS decision
         FROM observations WHERE trace_id=?1 AND call_id=?2 AND kind='replay' AND json_extract(detail,'$.edit')='individual'
         AND json_extract(detail,'$.sources[0].id')=?3 AND json_type(detail,'$.response_pair.changed') IN ('true','false')
         AND json_extract(detail,'$.measurement.location.decision_id') IN (SELECT value FROM json_each(?4))),
         ranked AS (SELECT *,count(*) OVER (PARTITION BY changed) AS reports,
         row_number() OVER (PARTITION BY changed ORDER BY n) AS sample FROM trials)
         SELECT json_object('changed',json(CASE changed WHEN 1 THEN 'true' ELSE 'false' END),'reports',reports,
            'group',ranked.group_id,'replay',replay,'decision',decision,'pair',json_extract(o.detail,'$.response_pair'))
         FROM ranked JOIN observations o ON o.trace_id=?1 AND o.n=ranked.n WHERE sample=1 ORDER BY changed DESC")?;
    let mut sources = Vec::new();
    for source in detail["sources"]
        .as_array()
        .ok_or("missing conflict sources")?
    {
        let id = source["id"].as_str().ok_or("missing source ID")?;
        let comparisons = query
            .query_map(params![trace, call_id, id, ids], |row| {
                row.get::<_, String>(0)
            })?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<Vec<Value>>>()?;
        let captured = call
            .context
            .iter()
            .any(|i| i.id == id && i.availability == Availability::Included && i.text.is_some());
        let matches = fields
            .iter()
            .find(|f| f.context_id == id)
            .map_or(0, |f| f.fields.len());
        sources.push(
            json!({"id":id,"captured":captured,"matching_fields":body.as_ref().map(|_| matches),
            "comparisons":comparisons}),
        );
    }
    Ok(json!({"recorded_request":body.is_some(),"sources":sources,
        "decisions":decisions.iter().map(|d| json!({"id":d.id,"label":d.label})).collect::<Vec<_>>()}))
}

// A conversation with a tested and an untested call still has a gap. Keep source
// factors separate since an installation can receive the same rule from multiple organizations.
pub(super) const COVERAGE: &str = ", conflict_calls AS (
    SELECT group_id,factor,thread_key,thread,observed,trace_id,call_id,n,
        CASE WHEN json_extract(detail,'$.conflict_trials.same_report') THEN 2
             WHEN json_array_length(detail,'$.conflict_trials.sources')>0 THEN 1 ELSE 0 END AS tested
    FROM events WHERE kind='conflict' AND thread_key IS NOT NULL
), conflict_threads AS (
    SELECT group_id,factor,thread_key,count(*) AS calls,max(observed) AS latest,
        CASE WHEN min(tested)=2 THEN 'complete' WHEN max(tested)=0 THEN 'untested' ELSE 'partial' END AS coverage
    FROM conflict_calls GROUP BY group_id,factor,thread_key
), conflict_groups AS (
    SELECT group_id,factor,count(*) AS conversations,sum(calls) AS calls,max(latest) AS latest,
        sum(coverage='complete') AS complete,sum(coverage='untested') AS untested,sum(coverage='partial') AS partial
    FROM conflict_threads GROUP BY group_id,factor
)";

pub(super) const SUMMARY: &str =
    "SELECT json_object('group',group_id,'factor',factor,'conversations',conversations,
    'calls',calls,'complete',complete,'untested',untested,'partial',partial,
    'conflict',json((SELECT json_object('rule',json_extract(e.detail,'$.rule'),'scope',json_extract(e.detail,'$.scope'),
        'sources',json_extract(e.detail,'$.sources'),'org',e.org) FROM events e
        WHERE (e.group_id,e.factor)=(g.group_id,g.factor) AND e.thread_key IS NOT NULL
        ORDER BY e.observed DESC,e.trace_id,e.n LIMIT 1)))
    FROM conflict_groups g";

pub(super) const CONVERSATIONS: &str = ", page AS (
    SELECT * FROM conflict_threads WHERE coverage=? ORDER BY latest DESC,thread_key LIMIT ? OFFSET ?
), examples AS (
    SELECT c.*,p.calls,p.coverage,p.latest,row_number() OVER (
        PARTITION BY c.thread_key ORDER BY c.tested,c.observed DESC,c.trace_id,c.call_id,c.n) AS example
    FROM page p JOIN conflict_calls c USING(group_id,factor,thread_key)
) SELECT json_object('thread',e.thread,'thread_key',e.thread_key,'calls',e.calls,'coverage',e.coverage,
    'trace',e.trace_id,'call',e.call_id,'group',e.group_id,'factor',e.factor,'tested',e.tested,
    'sources',json_extract(o.detail,'$.sources'),'tested_sources',coalesce(json_extract(o.detail,'$.conflict_trials.sources'),json('[]')))
    FROM examples e JOIN observations o ON (o.trace_id,o.n)=(e.trace_id,e.n)
    WHERE example=1 ORDER BY e.latest DESC,e.thread_key";
