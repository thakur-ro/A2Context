use nexusweave::{preflight, replay::candidates};
use serde_json::Value;

const INPUT: &str = include_str!("../examples/runtime-comparison/pending-returns/request.json");
const CASE: &str = include_str!("../examples/runtime-comparison/pending-returns/case.json");

fn setup() -> (preflight::Request, candidates::Case) {
    (
        serde_json::from_str(INPUT).unwrap(),
        serde_json::from_str(CASE).unwrap(),
    )
}

fn scores(work: &candidates::ScoringRequest<'_>) -> candidates::ScoringResult {
    candidates::ScoringResult {
        version: 1,
        model_id: "test-model".into(),
        model_revision: "v1".into(),
        runtime: "test-scorer".into(),
        tokenizer_sha256: "a".repeat(64),
        assistant_prefix_token_ids: vec![10],
        candidate_token_ids: [vec![30, 9], vec![60, 9]],
        conditions: work
            .conditions
            .iter()
            .zip([[-1.0, -5.0], [-6.0, -1.0], [-1.0, -8.0], [-1.5, -5.5]])
            .map(|(condition, log_probs)| candidates::ConditionScore {
                id: condition.id.clone(),
                rendered_prefix_sha256: "b".repeat(64),
                log_probs,
            })
            .collect(),
    }
}

#[test]
fn scores_fixed_candidates_in_one_batch_without_a_recorded_output() {
    let (input, case) = setup();
    let body = input.attributes["nexusweave.request.body"]
        .as_str()
        .unwrap();
    let original: Value = serde_json::from_str(body).unwrap();
    let report = candidates::execute(input, &case, |work| {
        assert_eq!(work.assistant_prefix, "The return window is");
        assert_eq!(work.candidates[1].continuation, " 60 days.");
        assert_eq!(work.conditions.len(), 4);
        assert_eq!(work.conditions[0].request, original);
        let baseline = original["messages"][0]["content"].as_str().unwrap();
        for (condition, (source, before, after)) in work.conditions[1..].iter().zip([
            (
                "policy",
                "Company policy: 30 days.",
                "Company policy: Not specified.",
            ),
            ("faq", "Archived FAQ: 60 days.", ""),
            ("store-name", "Store name: Alder.", "Store name: Rowan."),
        ]) {
            let mut expected = original.clone();
            expected["messages"][0]["content"] = baseline.replacen(before, after, 1).into();
            assert_eq!(condition.id, format!("edit:request-source:{source}"));
            assert_eq!(condition.request, expected);
        }
        Ok(scores(work))
    })
    .unwrap();
    assert_eq!(report.baseline_log_odds, 4.0);
    assert_eq!(report.effects[0].log_odds, -5.0);
    assert_eq!(report.effects[0].change_log_odds, -9.0);
    assert_eq!(
        report.effects[0].source_id.as_deref(),
        Some("retail/returns-policy")
    );
    assert_eq!(report.effects[0].evidence.field, "nexusweave.request.body");
    assert_eq!(report.effects[1].change_log_odds, 3.0);
    assert_eq!(report.effects[2].change_log_odds, 0.0);
    assert_ne!(
        report.effects[0].request_sha256,
        report.effects[1].request_sha256
    );
}

#[test]
fn checks_source_anchors_and_candidate_boundaries_before_scoring() {
    let (input, mut case) = setup();
    case.interventions[0].pointer = "/messages/1/content".into();
    let error =
        candidates::execute(input, &case, |_| panic!("Unexpected scoring call")).unwrap_err();
    assert!(error.contains("must match its declared fragment"));

    let (input, mut case) = setup();
    case.candidates[1].continuation = " 30 days. Unless the FAQ applies.".into();
    let error =
        candidates::execute(input, &case, |_| panic!("Unexpected scoring call")).unwrap_err();
    assert!(error.contains("non-overlapping continuations"));
}

#[test]
fn rejects_overlapping_tokens_mismatched_conditions_and_invalid_probabilities() {
    let (input, case) = setup();
    let error = candidates::execute(input, &case, |work| {
        let mut result = scores(work);
        result.candidate_token_ids[1] = vec![30];
        Ok(result)
    })
    .unwrap_err();
    assert!(error.contains("disjoint candidate tokens"));

    let (input, case) = setup();
    let error = candidates::execute(input, &case, |work| {
        let mut result = scores(work);
        result.conditions.swap(0, 1);
        Ok(result)
    })
    .unwrap_err();
    assert!(error.contains("mismatched candidate scores for baseline"));

    let (input, case) = setup();
    let error = candidates::execute(input, &case, |work| {
        let mut result = scores(work);
        result.conditions[0].log_probs = [-0.1, -0.1];
        Ok(result)
    })
    .unwrap_err();
    assert!(error.contains("Invalid or mismatched candidate scores"));
}

#[test]
fn saved_model_reports_keep_the_original_request_and_condition_effects() {
    for (request, case, saved) in [
        (
            INPUT,
            CASE,
            include_str!("../examples/runtime-comparison/pending-returns/report.json"),
        ),
        (
            include_str!("../examples/runtime-comparison/pending-routing/request.json"),
            include_str!("../examples/runtime-comparison/pending-routing/case.json"),
            include_str!("../examples/runtime-comparison/pending-routing/report.json"),
        ),
    ] {
        let input = serde_json::from_str(request).unwrap();
        let case = serde_json::from_str(case).unwrap();
        let saved: Value = serde_json::from_str(saved).unwrap();
        let report = candidates::execute(input, &case, |_| {
            Ok(serde_json::from_value(saved["scores"].clone()).unwrap())
        })
        .unwrap();
        assert_eq!(
            report.captured_request_sha256,
            saved["captured_request_sha256"]
        );
        assert!(report.baseline_log_odds > 0.0);
        assert!(report.effects[0].log_odds < 0.0);
        for (effect, stored) in report
            .effects
            .iter()
            .zip(saved["effects"].as_array().unwrap())
        {
            assert_eq!(effect.condition_id, stored["condition_id"]);
            assert_eq!(effect.request_sha256, stored["request_sha256"]);
            assert!(
                (effect.change_log_odds - stored["change_log_odds"].as_f64().unwrap()).abs()
                    < 1e-10
            );
        }
        assert!(report.effects.last().unwrap().change_log_odds.abs() < 0.10);
    }
}
