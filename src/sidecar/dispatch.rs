//! Compare one dispatcher design across the conversations behind a source edit.

pub(super) const DESIGNS: &str = ", receipts AS (
    SELECT group_id,factor,thread_key,observed,
        json_extract(detail,'$.sandbox_dispatch.design') AS design,
        json_extract(detail,'$.sandbox_dispatch.outcome') AS outcome,
        json_object('sources',json_extract(detail,'$.sources'),'guard',json_extract(detail,'$.sandbox_dispatch.guard'),
            'runner',json_extract(detail,'$.sandbox_dispatch.runner'),'tool',json_extract(detail,'$.sandbox_dispatch.tool'),
            'target',json_extract(detail,'$.sandbox_dispatch.target')) AS setup
    FROM events WHERE thread_key IS NOT NULL AND json_type(detail,'$.sandbox_dispatch')='object'
), per_conversation AS (
    SELECT group_id,factor,design,thread_key,max(observed) AS latest,min(setup) AS setup,
        CASE WHEN min(outcome)!=max(outcome) THEN 'mixed' ELSE min(outcome) END AS outcome
    FROM receipts GROUP BY group_id,factor,design,thread_key
), designs AS (
    SELECT group_id,factor,design,max(latest) AS latest,min(setup) AS setup,count(*) AS tested,
        count(*) FILTER (WHERE outcome='matched_with_check') AS helped,
        count(*) FILTER (WHERE outcome='matched_without_check') AS regressed,
        count(*) FILTER (WHERE outcome='matched_both') AS matched_both,
        count(*) FILTER (WHERE outcome='missed_both') AS missed_both,
        count(*) FILTER (WHERE outcome='mixed') AS mixed
    FROM per_conversation GROUP BY group_id,factor,design
)
SELECT json_object('group',group_id,'factor',factor,'design',design,'tested',tested,
    'setup',json(setup),
    'outcomes',json_object('matched_with_check',helped,'matched_without_check',regressed,
        'matched_both',matched_both,'missed_both',missed_both,'mixed',mixed))
FROM designs ORDER BY regressed DESC,mixed DESC,helped DESC,tested DESC,latest DESC,group_id,factor,design
LIMIT ? OFFSET ?";

// A NULL outcome means this observation has no verified receipt for the selected design.
// Count each conversation once. Reports with conflicting outcomes stay visible as mixed.
pub(super) const COHORT: &str = ", dispatch_observations AS (
    SELECT *,CASE WHEN json_extract(detail,'$.sandbox_dispatch.design')=?
         THEN json_extract(detail,'$.sandbox_dispatch.outcome') END AS dispatch_outcome
    FROM events WHERE thread_key IS NOT NULL
), dispatch_conversations AS (
    SELECT thread_key,max(thread) AS thread,
        coalesce(max(observed) FILTER (WHERE dispatch_outcome IS NOT NULL),max(observed)) AS latest,
        CASE WHEN min(dispatch_outcome)!=max(dispatch_outcome) THEN 'mixed'
             ELSE coalesce(min(dispatch_outcome),'unmeasured') END AS outcome
    FROM dispatch_observations GROUP BY thread_key
)";

pub(super) const SUMMARY: &str = " SELECT json_object('outcome',outcome,'conversations',count(*))
    FROM dispatch_conversations GROUP BY outcome ORDER BY outcome LIMIT ? OFFSET ?";

// List a recent witness per observed outcome. A mixed conversation has separate exact links.
pub(super) const EXAMPLES: &str = ", page AS (
    SELECT * FROM dispatch_conversations WHERE outcome=?
    ORDER BY latest DESC,thread_key LIMIT ? OFFSET ?
), witnesses AS (
    SELECT d.thread_key,d.trace_id,d.call_id,d.dispatch_outcome,
           json_extract(d.detail,'$.report_id') AS report,
           row_number() OVER(PARTITION BY d.thread_key,d.dispatch_outcome
               ORDER BY d.observed DESC,d.trace_id,d.call_id,d.n) AS rank
    FROM dispatch_observations d JOIN page p ON p.thread_key=d.thread_key
    WHERE d.dispatch_outcome IS NOT NULL OR p.outcome='unmeasured'
)
SELECT json_object('thread',p.thread,'outcome',p.outcome,'observed',p.latest,
    'examples',json_group_array(json_object('trace',w.trace_id,'call',w.call_id,'report',w.report,
        'outcome',w.dispatch_outcome) ORDER BY w.dispatch_outcome))
FROM page p JOIN witnesses w ON w.thread_key=p.thread_key AND w.rank=1
GROUP BY p.thread_key ORDER BY p.latest DESC,p.thread_key";

pub(super) fn valid_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "matched_with_check"
            | "matched_without_check"
            | "matched_both"
            | "missed_both"
            | "mixed"
            | "unmeasured"
    )
}
