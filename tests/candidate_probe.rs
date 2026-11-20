use nexusweave::{inspect, langsmith, otlp, preflight, replay::candidates, turn_evidence, Run};
use serde_json::{json, Value};

const INPUT: &str = include_str!("../examples/runtime-comparison/pending-returns/request.json");
const CASE: &str = include_str!("../examples/runtime-comparison/pending-returns/case.json");
const ROUTING_TRACE: &str =
    include_str!("../examples/runtime-comparison/pending-routing/trace.json");
const PROBES: &str = "nexusweave.candidate_probes";

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
                steps: None,
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

fn trace() -> Value {
    serde_json::from_str(ROUTING_TRACE).unwrap()
}

fn turn(runs: &[Run], include_excerpts: bool) -> Value {
    turn_evidence::query(
        &turn_evidence::Request {
            run_id: runs[0].id.clone(),
            call_id: runs[0].calls[0].id.clone(),
            decision_id: None,
            offset: 0,
            limit: 1,
            include_excerpts,
        },
        runs,
        &[],
        true,
    )
    .unwrap()
}

#[test]
fn links_the_saved_routing_probe_to_its_recorded_sources_and_tool_choice() {
    let raw = trace();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/report.json"
    ))
    .unwrap();
    assert_eq!(raw["extra"]["metadata"][PROBES][0], original);
    let runs = langsmith::import(&raw).unwrap();
    let normalized = serde_json::to_string(&runs).unwrap();
    let runs: Vec<Run> = serde_json::from_str(&normalized).unwrap();
    let report = inspect(&runs).unwrap();
    let probes = &report.candidate_probes;
    assert_eq!(probes.len(), 1);
    assert_eq!(
        probes[0].evidence.field,
        "extra.metadata[\"nexusweave.candidate_probes\"]"
    );
    let candidates::Verification::Matched { measurement } = &probes[0].verification else {
        panic!("Expected a matched report")
    };
    assert_eq!(measurement.candidates[0].id, "stock");
    assert_eq!(measurement.candidates[1].id, "price");
    assert!(measurement.baseline_log_odds > 0.0);
    assert!(measurement.effects[0].log_odds < 0.0);
    assert_eq!(
        measurement.effects[0].context_id,
        "request-source:request-type"
    );
    assert_eq!(measurement.effects[0].pointer, "/messages/1/content");

    let public = turn(&runs, false);
    assert_eq!(public["candidate_probes"][0]["status"], "matched");
    assert_eq!(public["decision"]["label"], "stock");
    assert!(public["candidate_probes"][0]["prefix_excerpt"].is_null());
    assert!(public["candidate_probes"][0]["candidate_excerpts"].is_null());
    assert_eq!(public["rows"][0]["kind"], "candidate_probe_effect");
    assert!(public["rows"][0]["replacement_excerpt"].is_null());
    assert_eq!(public["next_offset"], 1);
    let excerpts = turn(&runs, true);
    assert_eq!(
        excerpts["candidate_probes"][0]["prefix_excerpt"]["text"],
        "<tool_call>\n{{\"name\": \""
    );
    assert_eq!(
        excerpts["rows"][0]["replacement_excerpt"]["text"],
        "Request type: price."
    );
}

#[test]
fn rejects_stale_requests_and_tampered_reports_without_hiding_recorded_decisions() {
    let mut changed_body = trace();
    let body = &mut changed_body["extra"]["metadata"]["nexusweave.request.body"];
    *body = json!(format!("{} ", body.as_str().unwrap()));
    let mut changed_effect = trace();
    changed_effect["extra"]["metadata"][PROBES][0]["effects"][0]["change_log_odds"] = json!(4.0);
    let mut changed_edit = trace();
    changed_edit["extra"]["metadata"][PROBES][0]["effects"][0]["intervention"]["replacement"] =
        json!("Request type: something else.");
    let mut changed_source = trace();
    changed_source["extra"]["metadata"]["nexusweave.context.semantics"]["request_sources"][0]
        ["source_version"] = json!("changed");
    let mut malformed = trace();
    malformed["extra"]["metadata"][PROBES] = json!("{incomplete");
    for (changed, issue) in [
        (changed_body, "recorded request differs"),
        (changed_effect, "differs from its candidate scores"),
        (changed_edit, "edited request"),
        (
            changed_source,
            "does not identify one captured context entry",
        ),
        (malformed, "unreadable"),
    ] {
        let runs = langsmith::import(&changed).unwrap();
        let view = turn(&runs, false);
        assert_eq!(view["candidate_probes"][0]["status"], "unverified");
        assert!(view["candidate_probes"][0]["issue"]
            .as_str()
            .unwrap()
            .contains(issue));
        assert_ne!(view["rows"][0]["kind"], "candidate_probe_effect");
        assert_eq!(view["decision"]["label"], "stock");
    }
}

#[test]
fn attaches_otlp_pending_requests_by_source_version_and_exact_fragment() {
    let mut raw = trace();
    let metadata = &mut raw["extra"]["metadata"];
    let semantics = &mut metadata["nexusweave.context.semantics"];
    semantics["request_sources"][0]["id"] = json!("otel-request-type");
    let attributes = [
        ("gen_ai.operation.name", "chat".to_owned()),
        ("nexusweave.context.semantics", semantics.to_string()),
        (
            "nexusweave.request.body",
            metadata["nexusweave.request.body"]
                .as_str()
                .unwrap()
                .to_owned(),
        ),
        (PROBES, metadata[PROBES].to_string()),
    ]
    .map(|(key, value)| json!({"key":key,"value":{"stringValue":value}}));
    let export = json!({"resourceSpans":[{"scopeSpans":[{"spans":[{
        "traceId":"11111111111111111111111111111111", "spanId":"2222222222222222", "attributes": attributes,
    }]}]}]});
    let runs = otlp::import(&export).unwrap();
    let view = turn(&runs, false);
    assert!(view["decision"].is_null());
    assert_eq!(view["candidate_probes"][0]["anchor"]["field"], PROBES);
    assert_eq!(view["candidate_probes"][0]["status"], "matched");
    assert_eq!(
        view["rows"][0]["context_id"],
        "request-source:otel-request-type"
    );
    assert_eq!(view["rows"][0]["source_id"], "request-type");
}
