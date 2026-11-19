use nexusweave::{
    context_screening::{self as screening, Assessment, Selection, Suggestion},
    inspect, otlp, preflight, turn_evidence, Run,
};
use serde_json::{json, Value};

const REQUEST: &str = include_str!("../examples/context-screening/request.json");
const EVIDENCE: &str = include_str!("../examples/context-screening/evidence.json");

fn request() -> preflight::Request {
    serde_json::from_str(REQUEST).unwrap()
}

fn runs() -> Vec<Run> {
    serde_json::from_str(EVIDENCE).unwrap()
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

#[test]
fn matches_an_unannotated_otlp_call_and_keeps_excerpts_opt_in() {
    let captured = runs();
    let saved = captured[0].calls[0]
        .context_screening
        .as_ref()
        .unwrap()
        .report
        .as_ref()
        .unwrap();
    let mut attrs = request().attributes;
    attrs.insert("gen_ai.operation.name".into(), json!("chat"));
    attrs.insert(
        "nexusweave.context_screening".into(),
        serde_json::to_value(saved).unwrap(),
    );
    let attributes: Vec<_> = attrs
        .into_iter()
        .map(|(key, value)| {
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            json!({"key":key,"value":{"stringValue":text}})
        })
        .collect();
    let imported = otlp::import(&json!({"resourceSpans":[{"scopeSpans":[{"spans":[{
        "traceId":"11111111111111111111111111111111", "spanId":"2222222222222222", "attributes": attributes,
    }]}]}]})).unwrap();
    assert_eq!(inspect(&imported).unwrap().findings.len(), 0);
    let summary = query(&imported, false);
    assert_eq!(summary["context_screening"]["status"], "matched");
    assert_eq!(summary["context_screening"]["compared_pairs"], 21);
    assert_eq!(summary["context_screening"]["flagged_pairs"], 7);
    assert_eq!(summary["rows"][0]["kind"], "context_screen_pair");
    assert_eq!(summary["rows"][0]["suggestion"], "exact_text_reuse");
    assert_eq!(
        summary["rows"][0]["inputs"][0]["anchor"]["message_index"],
        0
    );
    assert!(summary["rows"][0]["inputs"][0]["excerpt"].is_null());
    assert!(!summary.to_string().contains("Customers can view"));
    assert_eq!(
        query(&imported, true)["rows"][0]["inputs"][0]["excerpt"]["text"],
        "Customers can view their own receipts."
    );
}

#[test]
fn does_not_publish_findings_from_stale_text_or_rewritten_labels() {
    let mut stale = runs();
    stale[0].calls[0].context[0]
        .text
        .as_mut()
        .unwrap()
        .push(' ');
    let mut changed = runs();
    changed[0].calls[0]
        .context_screening
        .as_mut()
        .unwrap()
        .report
        .as_mut()
        .unwrap()
        .pairs[0]
        .suggestion = Some(Suggestion::PossibleConflict);
    for invalid in [stale, changed] {
        let result = query(&invalid, false);
        assert_eq!(result["context_screening"]["status"], "unverified");
        assert_ne!(result["rows"][0]["kind"], "context_screen_pair");
    }
}

fn query(runs: &[Run], include_excerpts: bool) -> Value {
    turn_evidence::query(
        &turn_evidence::Request {
            run_id: runs[0].id.clone(),
            call_id: runs[0].calls[0].id.clone(),
            decision_id: None,
            include_excerpts,
            offset: 0,
            limit: 1,
        },
        runs.iter(),
        &[],
        include_excerpts,
    )
    .unwrap()
}
