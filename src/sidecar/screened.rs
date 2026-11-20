//! Read-side views of classifier flags and the single-source replays attached to each call.
pub(super) const SCOPED: &str = ", flags AS (
    SELECT p.*,c.started,t.thread_key,t.org,t.user,t.thread FROM screened_pairs p
    JOIN selected t ON t.id=p.trace_id JOIN captured_calls c USING(trace_id,call_id)
    WHERE t.thread_key IS NOT NULL";

// Keep the predicate identical to the partial index on individual replay outcomes.
pub(super) const TESTED: &str = "), coverage AS (
    SELECT f.*,EXISTS(SELECT 1 FROM observations o WHERE o.trace_id=f.trace_id AND o.call_id=f.call_id
        AND o.kind='replay' AND json_extract(o.detail,'$.edit')='individual'
        AND json_extract(o.detail,'$.sources[0].id') IN (f.left_id,f.right_id)
        AND json_type(o.detail,'$.response_pair.changed') IN ('true','false')) AS tested
    FROM flags f)";

pub(super) const INDEX: &str =
    "CREATE INDEX IF NOT EXISTS observations_screen_edit ON observations(
    trace_id,call_id,json_extract(detail,'$.sources[0].id'))
    WHERE kind='replay' AND json_extract(detail,'$.edit')='individual'
    AND json_type(detail,'$.response_pair.changed') IN ('true','false')";

// Bound replay lookups to the visible groups. Ordering needs only flags and capture timestamps.
pub(super) const GROUPS: &str = ", groups AS (
    SELECT pattern,relation,count(DISTINCT thread_key) AS conversations,
        count(DISTINCT json_array(trace_id,call_id)) AS calls,max(started) AS latest
    FROM flags GROUP BY pattern,relation ORDER BY conversations DESC,calls DESC,relation,latest DESC,pattern LIMIT ? OFFSET ?
), ranked AS (
    SELECT p.*,row_number() OVER (PARTITION BY p.pattern,p.relation ORDER BY p.started DESC,p.trace_id,p.call_id,p.ordinal) AS example
    FROM groups g JOIN flags p USING(pattern,relation)
), edits AS (
    SELECT DISTINCT p.pattern,p.relation,p.thread_key,json_extract(i.detail,'$.digest') AS digest,
        json_extract(o.detail,'$.response_pair.changed') AS changed
    FROM groups g JOIN flags p USING(pattern,relation)
    JOIN observations o ON o.trace_id=p.trace_id AND o.call_id=p.call_id
    JOIN screened_inputs i ON (i.trace_id,i.call_id,i.input_id)=(p.trace_id,p.call_id,json_extract(o.detail,'$.sources[0].id'))
    WHERE o.kind='replay' AND json_extract(o.detail,'$.edit')='individual'
        AND json_extract(o.detail,'$.sources[0].id') IN (p.left_id,p.right_id)
        AND json_type(o.detail,'$.response_pair.changed') IN ('true','false')
), source_counts AS (
    SELECT pattern,relation,digest,sum(changed) AS changed,sum(1-changed) AS held
    FROM edits GROUP BY pattern,relation,digest
) SELECT json_object('pattern',g.pattern,'relation',g.relation,'conversations',g.conversations,
    'tested',(SELECT count(DISTINCT thread_key) FROM edits e WHERE (e.pattern,e.relation)=(g.pattern,g.relation)), 'calls',g.calls,
    'sources',json_array(json(l.detail),json(r.detail)), 'scorer',json_extract(c.screen,'$.scorer'),
    'source_outcomes',(SELECT json_group_object(digest,json_object('changed',changed,'held',held))
        FROM source_counts s WHERE (s.pattern,s.relation)=(g.pattern,g.relation)))
    FROM groups g JOIN ranked e ON (e.pattern,e.relation,e.example)=(g.pattern,g.relation,1)
    JOIN captured_calls c ON (c.trace_id,c.call_id)=(e.trace_id,e.call_id)
    JOIN screened_inputs l ON (l.trace_id,l.call_id,l.input_id)=(e.trace_id,e.call_id,e.left_id)
    JOIN screened_inputs r ON (r.trace_id,r.call_id,r.input_id)=(e.trace_id,e.call_id,e.right_id)
    ORDER BY g.conversations DESC,g.calls DESC,g.relation,g.latest DESC,g.pattern";

// Page exact occurrences first. A source can have both changed and matching proposals on the same call.
pub(super) const EXAMPLES: &str = ", page AS (
    SELECT * FROM coverage ORDER BY tested,started DESC,trace_id,call_id,ordinal LIMIT ? OFFSET ?
), trials AS (
    SELECT p.trace_id,p.call_id,p.ordinal,o.n,json_extract(o.detail,'$.sources[0].id') AS input,
        row_number() OVER (PARTITION BY p.trace_id,p.call_id,p.ordinal,json_extract(o.detail,'$.sources[0].id'),
            json_extract(o.detail,'$.response_pair.changed') ORDER BY o.n) AS example
    FROM page p JOIN observations o ON o.trace_id=p.trace_id AND o.call_id=p.call_id
    WHERE o.kind='replay' AND json_extract(o.detail,'$.edit')='individual'
        AND json_extract(o.detail,'$.sources[0].id') IN (p.left_id,p.right_id)
        AND json_type(o.detail,'$.response_pair.changed') IN ('true','false')
) SELECT json_object('thread',p.thread,'org',p.org,'user',p.user,'started',p.started,'trace',p.trace_id,'call',p.call_id,
    'sources',json_array(json(l.detail),json(r.detail)),'decision',json_extract(c.screen,'$.decision'),'last_user_text',json_extract(c.screen,'$.last_user_text'),
    'comparisons',(SELECT json_group_array(json_object('context',tr.input,'group',o.group_id,
        'decision',json_extract(o.detail,'$.measurement.location.decision_id'),'replay',json_extract(o.detail,'$.measurement.replay_id'),
         'runner',json_extract(o.detail,'$.runner'),'pair',json_extract(o.detail,'$.response_pair'),
        'replacement',(SELECT json_object('text',substr(json_extract(v.value,'$.intervention.replacement'),1,200),
            'truncated',json(CASE WHEN length(json_extract(v.value,'$.intervention.replacement'))>200 THEN 'true' ELSE 'false' END))
            FROM reports r JOIN json_each(r.body,'$.variants') v
            WHERE (r.trace_id,r.kind,r.call_id,r.decision_id,r.id)=(o.trace_id,'replay-reports',o.call_id,
                json_extract(o.detail,'$.measurement.location.decision_id'),json_extract(o.detail,'$.measurement.replay_id'))
                AND json_extract(v.value,'$.intervention.context_id')=tr.input LIMIT 1)))
        FROM trials tr JOIN observations o ON (o.trace_id,o.n)=(tr.trace_id,tr.n)
        WHERE (tr.trace_id,tr.call_id,tr.ordinal)=(p.trace_id,p.call_id,p.ordinal) AND tr.example<=2))
    FROM page p JOIN captured_calls c USING(trace_id,call_id)
    JOIN screened_inputs l ON (l.trace_id,l.call_id,l.input_id)=(p.trace_id,p.call_id,p.left_id)
    JOIN screened_inputs r ON (r.trace_id,r.call_id,r.input_id)=(p.trace_id,p.call_id,p.right_id)
    ORDER BY p.tested,p.started DESC,p.trace_id,p.call_id,p.ordinal";
