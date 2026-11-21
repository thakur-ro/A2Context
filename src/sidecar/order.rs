//! Match two replay designs that exchange the same two source messages.

use a2context::{replay, Call};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn position(pointer: &str) -> Option<usize> {
    let index = pointer
        .strip_prefix("/messages/")?
        .strip_suffix("/content")?;
    let parsed = index.parse::<usize>().ok()?;
    (index == parsed.to_string()).then_some(parsed)
}

/// The caller has validated the report against the recorded request. Compare the
/// two single edits from a joint design, binding each edit to its stable source.
pub(super) fn trial(call: &Call, report: &replay::Replay) -> Option<Value> {
    let [a, b] = report.variants.as_slice() else {
        return None;
    };
    report.pair.as_ref()?;
    if report.control_comparison.is_some() || report.rescored_from.is_some() {
        return None;
    }
    let mut arms = Vec::with_capacity(2);
    for variant in [a, b] {
        variant.difference_pp?;
        let edit = &variant.intervention;
        if !matches!(
            edit.operation,
            replay::Operation::ReplaceText | replay::Operation::ReplaceTextFragment
        ) {
            return None;
        }
        let item = call.context.iter().find(|row| row.id == edit.context_id)?;
        let index = position(&edit.pointer)?;
        let anchored = match &item.request_fragment {
            Some(fragment) => {
                fragment.pointer == edit.pointer && edit.text_range == Some(fragment.text_range)
            }
            None => item.evidence.message_index == Some(index),
        };
        if !anchored {
            return None;
        }
        arms.push((
            item.source_id.as_deref()?,
            item.source_version.as_deref(),
            index,
            variant,
        ));
    }
    arms.sort_unstable_by_key(|arm| arm.0);
    let (first, second) = (arms[0].2, arms[1].2);
    let mut request: Value = serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
    let messages = request.get_mut("messages")?.as_array_mut()?;
    if arms[0].0 == arms[1].0 || messages.get(first)? == messages.get(second)? {
        return None;
    }
    if first > second {
        messages.swap(first, second);
    }
    let mut edits = Vec::new();
    let mut results = Vec::new();
    for (source, version, _, variant) in arms {
        let edit = &variant.intervention;
        edits.push(json!([
            source,
            version,
            edit.operation,
            edit.text_range,
            edit.replacement,
            variant.summary.attempted
        ]));
        let mut result = super::replacement::arm(&variant.summary, &variant.trials);
        result["source"] = json!(source);
        result["id"] = json!(edit.context_id);
        result["replacement"] = json!(edit.replacement.chars().take(160).collect::<String>());
        results.push(result);
    }
    let design = json!([
        request,
        first.min(second),
        first.max(second),
        edits,
        call.model,
        report.runner_id,
        report.match_rule,
        report.sampling_design,
        report.recorded,
        report.baseline_summary
    ]);
    Some(
        json!({"design":format!("{:x}", Sha256::digest(design.to_string().as_bytes())),
        "reversed":first>second,"edits":results,"baseline":super::replacement::arm(&report.baseline_summary, &report.baseline)}),
    )
}

// Each orientation contributes once. Conflicting repeats prevent selecting a
// convenient pair of trials. Factor and scope filters have already been applied.
pub(super) const MATCHED: &str = ", ordered AS (
    SELECT factor, json_extract(detail,'$.order_test.design') AS design,
        json_extract(detail,'$.order_test.reversed') AS reversed,
        json_extract(detail,'$.order_test.edits[0].matched') AS m0,
        json_extract(detail,'$.order_test.edits[1].matched') AS m1,
        json_object('trace',trace_id,'call',call_id,'thread',thread,
            'decision',json_extract(detail,'$.measurement.location.decision_id'),
            'report',json_extract(detail,'$.measurement.replay_id'),
            'edits',json_extract(detail,'$.order_test.edits'),
            'model',json_extract(detail,'$.model'),
            'baseline',json_extract(detail,'$.order_test.baseline')) AS example
    FROM events WHERE kind='replay' AND json_extract(detail,'$.order_test.design') IS NOT NULL
), orientations AS (
    SELECT factor,design,reversed,min(example) AS example,min(m0) AS m0,min(m1) AS m1
    FROM ordered GROUP BY factor,design,reversed HAVING min(m0)=max(m0) AND min(m1)=max(m1)
), pairs AS (
    SELECT a.factor,a.design,a.example AS forward,b.example AS reverse,
        (a.m0!=b.m0 OR a.m1!=b.m1) AS differs FROM orientations a JOIN orientations b
        ON a.factor=b.factor AND a.design=b.design AND a.reversed=0 AND b.reversed=1
)";
