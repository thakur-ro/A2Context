//! Classify source-edit findings by the outcome actually measured on a captured call.
use super::findings::{source_set_id, Observation};
use a2context::{replay, Run};
use serde_json::{json, Value};

pub(super) fn preview(choice: &replay::Choice) -> Value {
    let text = choice
        .outcome
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| choice.outcome.to_string());
    let mut chars = text.chars();
    let mut preview: String = chars.by_ref().take(160).collect();
    let truncated = chars.next().is_some();
    if truncated {
        preview.push('…');
    }
    json!({"kind":choice.kind,"label":choice.label,"outcome_preview":preview,"truncated":truncated})
}

pub(super) fn consistent(trials: &[replay::Trial]) -> Option<&replay::Choice> {
    let first = trials.first()?;
    match first {
        replay::Trial::Completed { choice } if trials.iter().all(|t| t == first) => Some(choice),
        _ => None,
    }
}

/// Store a pair only when every trial completed and agreed within each arm.
pub(super) fn compare(baseline: &[replay::Trial], variant: &[replay::Trial]) -> Option<Value> {
    let original = consistent(baseline)?;
    let edited = consistent(variant)?;
    Some(
        json!({"changed":original!=edited, "original":preview(original), "edited":preview(edited),
        "baseline_trials":baseline.len(), "edited_trials":variant.len()}),
    )
}

/// Compare the complete proposals first. Keep the match-rule measurement independent.
pub(super) fn joint_pattern(report: &replay::Replay) -> Option<&'static str> {
    let [first, second] = report.variants.as_slice() else {
        return None;
    };
    let baseline = consistent(&report.baseline)?;
    let a = consistent(&first.trials)? != baseline;
    let b = consistent(&second.trials)? != baseline;
    let joint = consistent(&report.pair.as_ref()?.trials)? != baseline;
    Some(match (a || b, joint) {
        (false, true) => "joint_only",
        (true, true) => "single_and_joint",
        (true, false) => "single_only",
        (false, false) => "unchanged",
    })
}

/// Keep only edit arms with a completed, changed proposal and an inspectable witness.
pub(super) fn changed_edits(run: &Run, observation: &Observation) -> Vec<Value> {
    let detail = &observation.detail;
    let sources = detail["sources"].as_array().expect("finding sources");
    if detail["response_pair"]["changed"] == true {
        if observation.kind == "source_mask" {
            return vec![json!({"id":detail["factor"],"sources":sources,"pair":true,
                "tool_changed":detail["mask"]["outcome"] == "changed_tool"})];
        }
        return vec![json!({"id":detail["factor"],"sources":sources,"pair":true})];
    }
    // The joint comparison can qualify through a changed single arm while the joint edit held.
    if detail["edit"] == "joint" {
        return detail["single_edits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|arm| arm["changed"] == true)
            .filter_map(|arm| sources.iter().find(|source| source["id"] == arm["id"]))
            .map(|source| {
                let edited = std::slice::from_ref(source);
                json!({"id":source_set_id(run,edited,None),"sources":edited,"arm":source["id"]})
            })
            .collect();
    }
    if observation.kind != "prefill" {
        return vec![];
    }
    let routing = &detail["routing_test"];
    let mut edits = Vec::new();
    for source in sources {
        let setup = routing["cells"].as_array().and_then(|cells| {
            cells
                .iter()
                .position(|cell| routing["context_id"] == source["id"] && cell["changed"] == true)
        });
        let check = detail["proposal_checks"]
            .as_array()
            .and_then(|checks| {
                checks
                    .iter()
                    .find(|check| check["context_id"] == source["id"] && check["changed"] == true)
            })
            .and_then(|check| check["report"].as_str());
        if setup.is_some() || check.is_some() {
            let edited = std::slice::from_ref(source);
            let tool_changed = setup.and_then(|i| routing["cells"][i]["tool_changed"].as_bool());
            edits.push(
                json!({"id":source_set_id(run,edited,None),"sources":edited,"setup":setup,
                "tool_changed":tool_changed,"check":check}),
            );
        }
    }
    edits
}

/// Link to one complete matching pair in the finding.
pub(super) fn held_witness(detail: &Value) -> Option<Value> {
    if detail["response_pair"]["changed"] == false {
        return Some(json!({"pair":true}));
    }
    let held = |row: &&Value| row["changed"] == false;
    if let Some(arm) = detail["single_edits"]
        .as_array()
        .and_then(|a| a.iter().find(held))
    {
        return Some(json!({"arm":arm["id"]}));
    }
    if let Some(cells) = detail["routing_test"]["cells"].as_array() {
        if let Some(index) = cells.iter().position(|row| row["changed"] == false) {
            return Some(json!({"setup":index}));
        }
    }
    let check = detail["proposal_checks"].as_array()?.iter().find(held)?;
    Some(json!({"check":check["report"],"context":check["context_id"]}))
}

pub(super) const THREAD_OUTCOMES: &str = ", by_thread AS (
    SELECT thread_key,max(thread) AS thread,max(observed) AS latest,
        max(kind='replay' AND changed=1) AS shifted,
        max(kind='replay' AND json_extract(detail,'$.effect_pp')=0) AS held,
        max(proposal_changed) AS proposal_shifted,
        max(json_type(detail,'$.held_witness')='object') AS proposal_held
    FROM events WHERE thread_key IS NOT NULL GROUP BY thread_key
), thread_results AS (
    SELECT *,CASE WHEN proposal_shifted AND proposal_held THEN 'both' WHEN proposal_shifted THEN 'changed_only'
      WHEN proposal_held THEN 'held_only' ELSE 'no_pair' END AS result FROM by_thread
)";

// Page conversations before choosing the most recent example for each applicable outcome.
pub(super) const PROPOSAL_CONVERSATIONS: &str = ", page AS (
    SELECT * FROM thread_results WHERE result=? ORDER BY latest DESC,thread_key LIMIT ? OFFSET ?
), candidates AS (
    SELECT e.thread_key,e.trace_id,e.n,k.value AS outcome,row_number() OVER (PARTITION BY e.thread_key,k.value
        ORDER BY e.observed DESC,e.trace_id,e.call_id,e.n) AS rank
    FROM page p JOIN events e ON e.thread_key=p.thread_key
    JOIN json_each('[\"changed\",\"held\",\"no_pair\"]') k ON (k.value='changed' AND e.proposal_changed)
        OR (k.value='held' AND json_type(e.detail,'$.held_witness')='object') OR (k.value='no_pair' AND p.result='no_pair')
) SELECT json_object('thread',p.thread,'user',max(t.user),'org',max(t.org),'latest',p.latest,
    'examples',json_group_object(c.outcome,json_object('trace',o.trace_id,'call',o.call_id,'group',o.group_id,
        'factor',json_extract(o.detail,'$.factor'),'report',json_extract(o.detail,'$.report_id'),'decision',json_extract(o.detail,'$.measurement.location.decision_id'),
        'replay',json_extract(o.detail,'$.measurement.replay_id'),'context',json_extract(o.detail,'$.measurement.context_id'),
        'witness',CASE c.outcome WHEN 'held' THEN json_extract(o.detail,'$.held_witness')
            WHEN 'changed' THEN json_extract(o.detail,'$.changed_edits[0]') END)))
    FROM page p JOIN candidates c ON c.thread_key=p.thread_key AND c.rank=1
    JOIN observations o ON (o.trace_id,o.n)=(c.trace_id,c.n) JOIN traces t ON t.id=c.trace_id
    GROUP BY p.thread_key ORDER BY p.latest DESC,p.thread_key";

// Keep the sort keys small. Load the selected finding's detail after pagination.
pub(super) const CLASSIFIED: &str = ", classified AS (
    SELECT thread_key,trace_id,call_id,n,observed,factor,
      CASE WHEN proposal_changed=1 THEN 1
           WHEN kind='replacement' OR (kind IN ('replay','shadow') AND json_extract(detail,'$.effect_pp')!=0) THEN 2
           WHEN kind IN ('candidate','token') OR (kind='prefill' AND json_extract(detail,'$.shifted')=1) THEN 3
           WHEN kind IN ('replay','shadow','prefill','source_mask') THEN 4
           ELSE 5 END AS level
    FROM events e WHERE thread_key IS NOT NULL
)";

/// Keep all observations at a conversation's strongest level. Filtering by source
/// after this step preserves membership in the original evidence cohort.
pub(super) const COHORT: &str = ", cohort AS (
    SELECT * FROM (
        SELECT c.*,min(level) OVER (PARTITION BY thread_key) AS strongest FROM classified c
    ) WHERE level=strongest AND strongest=CAST(? AS INTEGER)
)";

pub(super) const SOURCE_MEMBERS: &str = ", members AS (
    SELECT c.*,c.factor AS member_id,json_extract(o.detail,'$.sources') AS member_sources,NULL AS witness
    FROM cohort c JOIN observations o ON (o.trace_id,o.n)=(c.trace_id,c.n))";

pub(super) const EDIT_MEMBERS: &str = ", members AS (
    SELECT c.*,json_extract(a.value,'$.id') AS member_id,json_extract(a.value,'$.sources') AS member_sources,a.value AS witness
    FROM cohort c JOIN observations o ON (o.trace_id,o.n)=(c.trace_id,c.n)
    JOIN json_each(o.detail,'$.changed_edits') a)";

pub(super) const FACTORS: &str = " SELECT json_object('factor',c.member_id,
    'sources',json(min(c.member_sources)),'org',t.org,
    'conversations',count(DISTINCT c.thread_key),'requests',count(DISTINCT json_array(c.trace_id,c.call_id)),
    'cohort_size',(SELECT count(DISTINCT thread_key) FROM cohort))
    FROM members c JOIN traces t ON t.id=c.trace_id GROUP BY c.member_id,t.org
    ORDER BY count(DISTINCT c.thread_key) DESC,count(DISTINCT json_array(c.trace_id,c.call_id)) DESC,c.member_id LIMIT ? OFFSET ?";

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> replay::Trial {
        replay::Trial::Completed {
            choice: replay::Choice {
                kind: "text".into(),
                label: "answer".into(),
                outcome: json!(text),
            },
        }
    }

    #[test]
    fn compares_full_outcomes_before_clipping_the_preview() {
        let prefix = "猫".repeat(161);
        let original = answer(&prefix);
        let edited = answer(&(prefix + "A"));
        let pair = compare(&[original.clone(), original], &[edited]).unwrap();
        assert_eq!(pair["changed"], true);
        assert_eq!(pair["original"], pair["edited"]);
        assert_eq!(pair["original"]["truncated"], true);
        assert_eq!(pair["baseline_trials"], 2);
    }

    #[test]
    fn requires_complete_and_consistent_trials_in_both_arms() {
        let trials = [answer("A"), answer("A")];
        assert_eq!(compare(&trials, &trials).unwrap()["changed"], false);
        for unavailable in [
            vec![],
            vec![answer("A"), replay::Trial::Failed],
            vec![answer("A"), answer("B")],
        ] {
            assert!(compare(&unavailable, &trials).is_none());
            assert!(compare(&trials, &unavailable).is_none());
        }
    }
}
