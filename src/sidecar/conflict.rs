//! Resolve the evidence and editable fields for one declared disagreement on one call.
use super::findings::{digest, Observation};
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

// Only the last plain user message may vary. Keep all earlier dialogue, model
// settings and tool definitions byte-for-byte equivalent after JSON normalization.
fn question_setup(call: &Call) -> Option<String> {
    let mut body: Value = serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
    let last = body["messages"].as_array_mut()?.last_mut()?;
    let user = super::capture::last_user_message(call)?;
    let question = last["content"].as_str()?;
    if last["role"] != "user"
        || user.availability != Availability::Included
        || user.text.as_deref() != Some(question)
    {
        return None;
    }
    last["content"] = Value::Null;
    body.sort_all_objects();
    Some(digest(body.to_string()))
}

/// Each source must have its own edit in the same decision and report.
pub(super) fn annotate(run: &Run, structural: &mut [Observation], measurements: &[Observation]) {
    if !structural.iter().any(|f| f.kind == "conflict") {
        return;
    }
    let mut measured = BTreeMap::new();
    for (index, finding) in measurements.iter().enumerate().filter(|(_, f)| {
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
                .entry(finding.call_id.as_str())
                .or_insert_with(BTreeMap::new)
                .entry((decision, report))
                .or_insert_with(BTreeMap::new)
                .insert(source, (structural.len() + index, finding));
        }
    }
    let mut setups = BTreeMap::new();
    for finding in structural.iter_mut().filter(|f| f.kind == "conflict") {
        let Some(trials) = measured.get(finding.call_id.as_str()) else {
            continue;
        };
        let Some(call) = run.calls.iter().find(|c| c.id == finding.call_id) else {
            continue;
        };
        let sources = &finding.detail["sources"];
        let mut individual = BTreeSet::new();
        let mut together = false;
        let mut pairs = Vec::new();
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
                .filter_map(|id| Some((id, edits.get(id)?)))
                .collect();
            individual.extend(matched.iter().map(|(id, _)| *id));
            let [(_, (a, left)), (_, (b, right))] = matched.as_slice() else {
                continue;
            };
            together = true;
            let setup = setups
                .entry(call.id.as_str())
                .or_insert_with(|| question_setup(call));
            if let (Some(setup), Some(input), Some(ls), Some(rs)) = (
                setup.as_ref(),
                left.detail["input_digest"].as_str(),
                left.detail["prior"]["setup"].as_str(),
                right.detail["prior"]["setup"].as_str(),
            ) {
                let design = digest(json!([setup, left.group, right.group, ls, rs]).to_string());
                let pattern = match (
                    left.detail["response_pair"]["changed"] == true,
                    right.detail["response_pair"]["changed"] == true,
                ) {
                    (true, false) => "left",
                    (false, true) => "right",
                    (true, true) => "both",
                    (false, false) => "neither",
                };
                pairs.push(json!({"design":design,"input":input,"pattern":pattern,"arms":[a,b]}));
            }
        }
        if !individual.is_empty() {
            finding.detail["conflict_trials"] =
                json!({"sources":individual,"same_report":together,"pairs":pairs});
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

// Count unique requests. Opposite results on repeats of one request cannot
// supply either side of a cross-question comparison.
pub(super) const PATTERNS: &str = ", paired AS (
    SELECT e.group_id,e.factor,e.trace_id,e.call_id,e.n,e.thread_key,e.thread,e.observed,
        p.value->>'design' AS design,p.value->>'input' AS input,p.value->>'pattern' AS pattern,p.value->'arms' AS arms
    FROM events e,json_each(e.detail,'$.conflict_trials.pairs') p
    WHERE e.kind='conflict' AND e.thread_key IS NOT NULL
), by_question AS (
    SELECT group_id,factor,design,input,min(thread_key) AS first_thread,max(thread_key) AS last_thread,
        CASE WHEN min(pattern)=max(pattern) THEN min(pattern) ELSE 'mixed' END AS pattern
    FROM paired GROUP BY group_id,factor,design,input
), contrasting AS (
    SELECT group_id,factor,design,count(*) AS inputs,sum(pattern='left') AS left_count,sum(pattern='right') AS right_count
    FROM by_question GROUP BY group_id,factor,design
    HAVING left_count>0 AND right_count>0 AND min(first_thread) FILTER (WHERE pattern IN ('left','right')) <>
        max(last_thread) FILTER (WHERE pattern IN ('left','right'))
)";

pub(super) const TOP_PATTERN: &str = "SELECT json_object('group',g.group_id,'factor',g.factor,'design',g.design,
    'inputs',g.inputs,'left_count',g.left_count,'right_count',g.right_count,'sources',
    (SELECT json_extract(e.detail,'$.sources') FROM events e WHERE (e.group_id,e.factor)=(g.group_id,g.factor) LIMIT 1))
    FROM contrasting g ORDER BY min(left_count,right_count) DESC,left_count+right_count DESC,group_id,factor,design LIMIT 1";

// Fetch proposal text only for the visible page. The ordinals refer to the
// two observations written in the same transaction as the conflict.
pub(super) const EXAMPLES: &str = ", candidates AS (
    SELECT p.*,q.pattern AS result,row_number() OVER (PARTITION BY p.input ORDER BY p.observed DESC,p.trace_id,p.n,p.arms) AS sample
    FROM paired p JOIN by_question q USING(group_id,factor,design,input)
    WHERE p.design=?
), examples AS (SELECT *,row_number() OVER (PARTITION BY result ORDER BY input) AS ordinal
    FROM candidates WHERE sample=1
), page AS (SELECT * FROM examples ORDER BY result NOT IN ('left','right'),ordinal,result,input LIMIT ? OFFSET ?)
    SELECT json_object('pattern',p.result,'thread',p.thread,'trace',p.trace_id,'call',p.call_id,
        'question',json_extract(c.preview,'$.user.text'),'runner',a.detail->>'runner',
        'replay',a.detail->>'$.measurement.replay_id','decision',a.detail->>'$.measurement.location.decision_id',
        'arms',json_array(json_object('group',a.group_id,'source',a.detail->'$.sources[0]','pair',a.detail->'response_pair','replacement',a.detail->>'$.prior.replacement'),
                          json_object('group',b.group_id,'source',b.detail->'$.sources[0]','pair',b.detail->'response_pair','replacement',b.detail->>'$.prior.replacement')))
    FROM page p JOIN conversation_calls c USING(trace_id,call_id)
    JOIN observations a ON (a.trace_id,a.n)=(p.trace_id,p.arms->>0)
    JOIN observations b ON (b.trace_id,b.n)=(p.trace_id,p.arms->>1)
    ORDER BY p.result NOT IN ('left','right'),p.ordinal,p.result,p.input";
