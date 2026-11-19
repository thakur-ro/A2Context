use nexusweave::{
    inspect, langsmith, otlp, preflight,
    replay::{shadow, Choice, Trial},
    turn_evidence, Run,
};
use serde_json::{json, Value};

const REQUEST: &str = include_str!("../examples/runtime-comparison/pending-routing/request.json");
const CASE: &str = include_str!("../examples/runtime-comparison/pending-routing/shadow-case.json");
const TRACE: &str =
    include_str!("../examples/runtime-comparison/pending-routing/shadow-trace.json");
const SHADOW: &str = "nexusweave.shadow_probes";

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
    assert_eq!(report.effects[0].evidence.field, "nexusweave.request.body");
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

#[test]
fn recomputes_the_saved_qwen_report_from_its_proposals() {
    let saved: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/shadow-report.json"
    ))
    .unwrap();
    let baseline = &saved["baseline"][0]["choice"];
    let target = &saved["effects"][0]["trials"][0]["choice"];
    let control = &saved["effects"][1]["trials"][0]["choice"];
    let joint = &saved["pair"]["trials"][0]["choice"];
    assert_eq!(
        [baseline, target, control, joint].map(|item| item["label"].as_str().unwrap()),
        ["stock", "price", "stock", "price"]
    );
    let (request, case) = setup();
    let report = shadow::execute(request, &case, |request| {
        let choice = match condition(request) {
            (false, false) => baseline,
            (true, false) => target,
            (false, true) => control,
            (true, true) => joint,
        };
        Some(serde_json::from_value(choice.clone()).unwrap())
    })
    .unwrap();
    assert_eq!(serde_json::to_value(&report).unwrap(), saved);
    let loaded: shadow::Report = serde_json::from_value(saved).unwrap();
    assert_eq!(
        loaded.control_comparison.unwrap().target_minus_control_pp,
        Some(100.0)
    );
}

fn trace() -> Value {
    serde_json::from_str(TRACE).unwrap()
}

fn turn(runs: &[Run], offset: usize, include_excerpts: bool) -> Value {
    turn_evidence::query(
        &turn_evidence::Request {
            run_id: runs[0].id.clone(),
            call_id: runs[0].calls[0].id.clone(),
            decision_id: None,
            include_excerpts,
            offset,
            limit: 1,
        },
        runs,
        &[],
        include_excerpts,
    )
    .unwrap()
}

#[test]
fn matches_a_shadow_probe_to_a_stock_call_and_limits_visible_proposals() {
    let trace = trace();
    let saved: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/pending-routing/shadow-report.json"
    ))
    .unwrap();
    assert_eq!(trace["extra"]["metadata"][SHADOW][0], saved);
    let runs = langsmith::import(&trace).unwrap();
    let runs: Vec<Run> = serde_json::from_value(serde_json::to_value(runs).unwrap()).unwrap();
    let inspection = inspect(&runs).unwrap();
    let probe = &inspection.shadow_probes[0];
    assert_eq!(
        probe.evidence.field,
        "extra.metadata[\"nexusweave.shadow_probes\"]"
    );
    let shadow::Verification::Matched { measurement } = &probe.verification else {
        panic!("Expected a matched shadow proposal");
    };
    assert_eq!(measurement.expected.label, "price");
    assert_eq!(measurement.pair.as_ref().unwrap().interaction_pp, Some(0.0));
    assert_eq!(measurement.effects[0].difference_pp, Some(100.0));
    assert_eq!(measurement.effects[1].difference_pp, Some(0.0));

    let summary = turn(&runs, 0, false);
    assert_eq!(summary["decision"]["label"], "stock");
    assert_eq!(summary["shadow_probes"][0]["baseline"]["matched"], 0);
    assert!(summary["shadow_probes"][0]["expected_label"].is_null());
    assert!(summary["shadow_probes"][0]["expected_outcome"].is_null());
    assert!(summary["rows"][0]["replacement_excerpt"].is_null());
    let detail = turn(&runs, 0, true);
    assert_eq!(
        detail["shadow_probes"][0]["expected_label"]["text"],
        "price"
    );
    assert_eq!(detail["rows"][0]["kind"], "shadow_probe_effect");
    assert_eq!(
        detail["rows"][0]["replacement_excerpt"]["text"],
        "Request type: price."
    );
    assert_eq!(detail["rows"][0]["text_range"]["end_byte"], 20);
    assert_eq!(detail["next_offset"], 1);
    assert_eq!(turn(&runs, 1, true)["rows"][0]["source_id"], "store-name");
}

#[test]
fn rejects_shadow_reports_that_do_not_match_request_or_trial_evidence() {
    for (pointer, replacement, issue) in [
        (
            "/captured_request_sha256",
            json!("other"),
            "recorded request differs",
        ),
        (
            "/effects/0/source_version",
            json!("v2"),
            "does not identify",
        ),
        (
            "/effects/0/captured_context_sha256",
            json!("other"),
            "edited request",
        ),
        (
            "/effects/0/request_sha256",
            json!("other"),
            "edited request",
        ),
        ("/effects/0/trials", json!([]), "proposal trials"),
        ("/effects/0/difference_pp", json!(0), "proposal trials"),
        (
            "/effects/0/difference_95_interval_pp",
            json!({"lower":100,"upper":100}),
            "proposal trials",
        ),
        ("/baseline_summary/matched", json!(1), "baseline counts"),
        (
            "/pair/request_sha256",
            json!("other"),
            "shadow joint request",
        ),
        ("/pair/interaction_pp", json!(-1), "shadow joint request"),
        (
            "/control_comparison/target_minus_control_pp",
            json!(0),
            "control comparison",
        ),
    ] {
        let mut trace = trace();
        let report = &mut trace["extra"]["metadata"][SHADOW][0];
        if let Some(slot) = report.pointer_mut(pointer) {
            *slot = replacement;
        } else {
            let (parent, leaf) = pointer.rsplit_once('/').unwrap();
            report.pointer_mut(parent).unwrap()[leaf] = replacement;
        }
        let runs = langsmith::import(&trace).unwrap();
        let view = turn(&runs, 0, false);
        assert_eq!(
            view["shadow_probes"][0]["status"], "unverified",
            "{pointer}"
        );
        assert!(
            view["shadow_probes"][0]["issue"]
                .as_str()
                .unwrap()
                .contains(issue),
            "{pointer}: {view}"
        );
        assert_ne!(view["rows"][0]["kind"], "shadow_probe_effect");
    }

    let mut trace = trace();
    let reports = &mut trace["extra"]["metadata"][SHADOW];
    *reports = json!([reports[0], reports[0]]);
    let runs = langsmith::import(&trace).unwrap();
    let view = turn(&runs, 0, false);
    assert!(view["shadow_probes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|probe| probe["status"] == "unverified"));
}

#[test]
fn attaches_shadow_probes_to_pending_otlp_calls_with_imported_source_ids() {
    let raw = trace();
    let metadata = &raw["extra"]["metadata"];
    let mut semantics = metadata["nexusweave.context.semantics"].clone();
    semantics["request_sources"][0]["id"] = json!("imported-request");
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
        (SHADOW, metadata[SHADOW].to_string()),
    ]
    .map(|(key, value)| json!({"key":key,"value":{"stringValue":value}}));
    let export = json!({"resourceSpans":[{"scopeSpans":[{"spans":[{
        "traceId":"11111111111111111111111111111111", "spanId":"2222222222222222", "attributes": attributes,
    }]}]}]});
    let runs = otlp::import(&export).unwrap();
    let result = turn(&runs, 0, false);
    assert!(result["decision"].is_null());
    assert_eq!(result["shadow_probes"][0]["status"], "matched");
    assert_eq!(result["shadow_probes"][0]["anchor"]["field"], SHADOW);
    assert_eq!(
        result["rows"][0]["context_id"],
        "request-source:imported-request"
    );
}
