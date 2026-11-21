use a2context::{
    preflight,
    replay::{shadow, Choice, Trial},
};
use serde_json::{json, Value};

const REQUEST: &str = include_str!("../fixtures/routing-request.json");
const CASE: &str = include_str!("../fixtures/routing-shadow.json");
fn setup() -> (preflight::Request, shadow::Case) {
    (
        serde_json::from_str(REQUEST).unwrap(),
        serde_json::from_str(CASE).unwrap(),
    )
}

fn condition(request: &Value) -> (bool, bool) {
    let target = request["messages"][1]["content"] == "Request type: price.";
    let control = request["messages"][3]["content"]
        .as_str()
        .unwrap()
        .contains("Rowan.");
    assert_eq!(request["messages"][2]["content"], "Product ID: 42.");
    (target, control)
}

fn proposal(price: bool) -> Choice {
    Choice {
        kind: "tool_call".into(),
        label: if price { "price" } else { "stock" }.into(),
        outcome: json!({"product_id":42}),
    }
}

#[test]
fn measures_a_new_proposal_from_a_zero_match_baseline_and_rotates_trials() {
    let (request, mut case) = setup();
    case.samples = 2;
    let mut order = Vec::new();
    let report = shadow::execute(request, &case, |request| {
        let edit = condition(request);
        order.push(edit);
        Some(proposal(edit.0))
    })
    .unwrap();
    assert_eq!(
        order,
        [
            (false, false),
            (true, false),
            (false, true),
            (true, true),
            (true, false),
            (false, true),
            (true, true),
            (false, false),
        ]
    );
    assert_eq!(report.baseline_summary.completed, 2);
    assert_eq!(report.baseline_summary.matched, 0);
    assert_eq!(report.effects[0].result.summary.matched, 2);
    assert_eq!(report.effects[0].result.difference_pp, Some(100.0));
    assert_eq!(report.effects[0].source_id.as_deref(), Some("request-type"));
    assert_eq!(report.effects[0].evidence.field, "a2context.request.body");
    assert_eq!(report.effects[1].result.difference_pp, Some(0.0));
    assert_eq!(
        report.pair.as_ref().unwrap().result.interaction_pp,
        Some(0.0)
    );
    assert_eq!(
        report.control_comparison.unwrap().target_minus_control_pp,
        Some(100.0)
    );
}

#[test]
fn a_joint_edit_can_reveal_a_proposal_neither_source_edit_produces_on_its_own() {
    let (request, case) = setup();
    let report = shadow::execute(request, &case, |request| {
        let (target, control) = condition(request);
        Some(proposal(target && control))
    })
    .unwrap();
    assert_eq!(report.baseline_summary.matched, 0);
    assert!(report
        .effects
        .iter()
        .all(|effect| effect.result.difference_pp == Some(0.0)));
    let pair = report.pair.unwrap();
    assert_eq!(pair.result.difference_pp, Some(100.0));
    assert_eq!(pair.result.interaction_pp, Some(100.0));
}

#[test]
fn withholds_contrasts_for_incomplete_conditions_and_preserves_successful_trials() {
    let (request, case) = setup();
    let report = shadow::execute(request, &case, |request| match condition(request) {
        (true, false) => Some(Choice {
            outcome: json!({}),
            ..proposal(true)
        }),
        (false, true) => Some(Choice {
            label: " ".into(),
            ..proposal(false)
        }),
        (target, _) => Some(proposal(target)),
    })
    .unwrap();
    assert_eq!(report.effects[0].result.summary.unscored, 1);
    assert_eq!(report.effects[0].result.difference_pp, None);
    assert_eq!(report.effects[1].result.trials, [Trial::Failed]);
    assert_eq!(report.effects[1].result.difference_pp, None);
    let pair = report.pair.unwrap();
    assert_eq!(pair.result.difference_pp, Some(100.0));
    assert_eq!(pair.result.interaction_pp, None);
    assert_eq!(
        report.control_comparison.unwrap().target_minus_control_pp,
        None
    );
}

#[test]
fn rejects_invalid_captured_sources_and_outcome_rules_before_sampling() {
    let (request, case) = setup();
    shadow::validate_case(request, &case).unwrap();

    let (request, mut case) = setup();
    case.interventions[0].pointer = "/messages/2/content".into();
    let plan_error = shadow::validate_case(setup().0, &case).unwrap_err();
    let error = shadow::execute(request, &case, |_| panic!("Unexpected runner call")).unwrap_err();
    assert!(error.contains("must match its declared fragment"));
    assert_eq!(error, plan_error);

    let (request, mut case) = setup();
    case.expected.outcome = json!({"other_product":42});
    let plan_error = shadow::validate_case(setup().0, &case).unwrap_err();
    let error = shadow::execute(request, &case, |_| panic!("Unexpected runner call")).unwrap_err();
    assert!(error.contains("reference outcome has no field"));
    assert_eq!(error, plan_error);
}
