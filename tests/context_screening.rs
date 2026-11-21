use a2context::{
    context_screening::{self as screening, Assessment, Selection, Suggestion},
    preflight,
};
use serde_json::{json, Value};

const REQUEST: &str = include_str!("../fixtures/screening-request.json");
fn request() -> preflight::Request {
    serde_json::from_str(REQUEST).unwrap()
}

#[test]
fn exact_reuse_bypasses_the_scorer_and_the_report_covers_the_selected_pairs() {
    let selection = Selection {
        pair_limit: 2,
        ..Default::default()
    };
    let report = screening::screen_pending(request(), &selection, |batch| {
        assert_eq!(batch.pairs.len(), 1);
        assert_eq!(batch.pairs[0].id, 1);
        assert_ne!(batch.pairs[0].left, batch.pairs[0].right);
        Ok(None)
    })
    .unwrap();
    assert_eq!(report.total_pairs, 21);
    assert_eq!(report.pairs[0].suggestion, Some(Suggestion::ExactTextReuse));
    assert_eq!(
        report.pairs[1].assessment,
        Assessment::Unscored {
            reason: screening::Unscored::NoScorer
        }
    );
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("Customers can view"));

    let selected = Selection {
        context_ids: vec!["input:1:0".into(), "input:0:0".into()],
        ..Default::default()
    };
    let report = screening::screen_pending(request(), &selected, |_| {
        panic!("Same text bypasses the scorer")
    })
    .unwrap();
    assert_eq!(report.sources[0].context_id, "input:0:0");
    assert_eq!(report.total_pairs, 1);
    assert!(report.scorer.is_none());
    let bad = Selection {
        context_ids: vec!["missing".into()],
        ..Default::default()
    };
    assert!(screening::screen_pending(request(), &bad, |_| panic!("Invalid input")).is_err());
}

fn classifier_row(reversed: Value, id: usize) -> screening::ScoringResult {
    serde_json::from_value(json!({
        "version": 1,
        "scorer": {"model_id":"test-model", "revision":"test", "runtime":"test"},
        "pairs": [{
            "id": id, "status":"scored",
            "left_to_right":{"contradiction":0.9,"entailment":0.05,"neutral":0.05},
            "right_to_left": reversed,
        }],
    }))
    .unwrap()
}

#[test]
fn requires_agreement_and_rejects_misaligned_or_invalid_scores() {
    let selection = Selection {
        context_ids: vec!["input:0:0".into(), "input:3:0".into()],
        ..Default::default()
    };
    let neutral = json!({"contradiction":0.05,"entailment":0.05,"neutral":0.9});
    let report = screening::screen_pending(request(), &selection, |_| {
        Ok(Some(classifier_row(neutral.clone(), 0)))
    })
    .unwrap();
    assert_eq!(report.pairs[0].suggestion, None);

    let invalid = json!({"contradiction":0.9,"entailment":0.9,"neutral":0.9});
    assert!(screening::screen_pending(request(), &selection, |_| {
        Ok(Some(classifier_row(invalid, 0)))
    })
    .is_err());
    assert!(screening::screen_pending(request(), &selection, |_| {
        Ok(Some(classifier_row(neutral, 3)))
    })
    .is_err());
}
