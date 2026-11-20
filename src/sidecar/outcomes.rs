//! Classify source-edit findings by the outcome actually measured on a captured call.
use nexusweave::replay;
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

fn consistent(trials: &[replay::Trial]) -> Option<&replay::Choice> {
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

// Keep the sort keys small. Load the selected finding's detail after pagination.
pub(super) const CLASSIFIED: &str = ", classified AS (
    SELECT thread_key,trace_id,call_id,n,observed,
      CASE WHEN json_extract(detail,'$.response_pair.changed')=1 THEN 1
           WHEN kind='prefill' AND (EXISTS (SELECT 1 FROM json_each(e.detail,'$.proposal_checks') c WHERE json_extract(c.value,'$.changed')=1)
                 OR EXISTS (SELECT 1 FROM json_each(e.detail,'$.routing_test.cells') c WHERE json_extract(c.value,'$.changed')=1)) THEN 1
           WHEN kind='replacement' OR (kind IN ('replay','shadow') AND json_extract(detail,'$.effect_pp')!=0) THEN 2
           WHEN kind IN ('candidate','token') OR (kind='prefill' AND json_extract(detail,'$.shifted')=1) THEN 3
           WHEN kind IN ('replay','shadow','prefill') THEN 4
           ELSE 5 END AS level
    FROM events e WHERE thread_key IS NOT NULL
)";

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
