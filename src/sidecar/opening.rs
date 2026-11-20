//! Compare recorded opening-token readouts with proposal trials for the same edits.
use super::{findings::digest, outcomes};
use nexusweave::replay;
use serde_json::{json, Value};

pub(super) fn trial(baseline: &[replay::Trial], edited: &[replay::Trial]) -> Value {
    outcomes::compare(baseline, edited).unwrap_or_else(|| {
        let failed = baseline
            .iter()
            .chain(edited)
            .any(|t| matches!(t, replay::Trial::Failed));
        json!({"status":if failed { "incomplete" } else { "varied" }})
    })
}

pub(super) fn review(ranked: &[Value], checks: Vec<Value>) -> Value {
    let mut arms = Vec::new();
    for row in ranked.iter().filter(|r| r["opening_changed"] == false) {
        let matching_checks = || {
            checks
                .iter()
                .filter(|c| c["context_id"] == row["context_id"])
        };
        let tested = matching_checks().any(|c| c["changed"].is_boolean());
        let changed = matching_checks().any(|c| c["changed"] == true);
        let slot = digest(
            json!([
                row["source"],
                row["version"],
                row["pointer"],
                row["text_range"]
            ])
            .to_string(),
        );
        arms.push(json!({"slot":slot,"context":row["context_id"],"source":row["source"],"version":row["version"],
            "pointer":row["pointer"],"text_range":row["text_range"],"tested":tested,"affected":changed}));
    }
    json!({"tested":arms.iter().any(|a| a["tested"]==true),"affected":arms.iter().any(|a| a["affected"]==true),
        "arms":arms,"checks":checks})
}

// Only instrumented calls enter the denominator. Identified conversations count once
// even when a source has multiple reports, calls, or outcomes in the same thread.
pub(super) const CALLS: &str = ", opening_calls AS (
    SELECT group_id,factor,thread,thread_key,trace_id,call_id,n,observed,
        detail->>'$.opening.tested' AS tested,detail->>'$.opening.affected' AS affected,
        detail->>'baseline_request_sha256' AS input
    FROM events WHERE kind='prefill' AND thread_key IS NOT NULL AND json_type(detail,'$.opening')='object'
), opening_groups AS (
    SELECT group_id,factor,count(DISTINCT thread_key) AS scored,count(DISTINCT thread_key) FILTER (WHERE tested) AS tested,
        count(DISTINCT thread_key) FILTER (WHERE affected) AS affected,count(DISTINCT input) FILTER (WHERE affected) AS inputs,
        max(observed) FILTER (WHERE affected) AS latest
    FROM opening_calls GROUP BY group_id,factor
)";

pub(super) const SUMMARY: &str =
    "SELECT json_object('group',group_id,'factor',factor,'scored',scored,
    'tested',tested,'affected',affected,'affected_inputs',inputs) FROM opening_groups";

pub(super) const ARMS: &str = ", opening_arms AS (
    SELECT c.*,a.value AS arm,a.value->>'slot' AS slot FROM opening_calls c
    JOIN observations o ON (o.trace_id,o.n)=(c.trace_id,c.n) JOIN json_each(o.detail,'$.opening.arms') a
)";

pub(super) const SOURCES: &str = " SELECT json_object('slot',slot,'source',arm->>'source','version',arm->>'version',
    'pointer',arm->>'pointer','text_range',arm->'text_range','scored',count(DISTINCT thread_key),
    'tested',count(DISTINCT thread_key) FILTER (WHERE arm->>'tested'),
    'affected',count(DISTINCT thread_key) FILTER (WHERE arm->>'affected'))
    FROM opening_arms GROUP BY slot ORDER BY count(DISTINCT thread_key) FILTER (WHERE arm->>'affected') DESC,slot LIMIT ? OFFSET ?";

pub(super) const EXAMPLES: &str = ", witnesses AS (
    SELECT *,row_number() OVER (PARTITION BY thread_key ORDER BY observed DESC,trace_id,call_id,slot) AS sample
    FROM opening_arms WHERE arm->>'affected'";
pub(super) const PAGE: &str = ") SELECT json_object('thread',w.thread,'thread_key',w.thread_key,'trace',w.trace_id,
    'call',w.call_id,'question',c.preview->>'$.user.text','context',w.arm->>'context','source',w.arm->>'source')
    FROM witnesses w JOIN conversation_calls c USING(trace_id,call_id) WHERE sample=1
    ORDER BY w.observed DESC,w.thread_key LIMIT ? OFFSET ?";
