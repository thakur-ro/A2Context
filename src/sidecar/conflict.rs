//! Resolve the evidence and editable fields for one declared disagreement on one call.
use crate::Result;
use nexusweave::{replay, Availability, Call};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

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
        .filter(|d| {
            matches!(d.kind.as_str(), "model_output" | "tool_call")
                && !d.outcome.is_null()
                && d.applies_to
                    .as_ref()
                    .is_none_or(|s| detail["rule"] == s.key && detail["scope"] == s.scope)
        })
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
