use nexusweave::{replay, replay_effects::query, Run};
use serde_json::{json, Value};

const QWEN_EVIDENCE: &str = include_str!("../examples/context-screening/replay/evidence.json");
const QWEN_REPLAY: &str = include_str!("../examples/context-screening/replay/report.json");
const QWEN_SECOND: &str = include_str!("../examples/context-screening/replay/report-policy.json");
const QWEN_CASE: &str = include_str!("../examples/context-screening/replay/case.json");

fn read(request: Value, runs: &[Run], reports: &[replay::Replay]) -> Value {
    query::query(
        &serde_json::from_value(request).unwrap(),
        runs,
        reports,
        false,
    )
    .unwrap()
}

#[test]
fn compares_exact_groups_and_pages_each_scope_in_recorded_order() {
    let runs: Vec<Run> = serde_json::from_str(QWEN_EVIDENCE).unwrap();
    let reports = [
        serde_json::from_str(QWEN_SECOND).unwrap(),
        serde_json::from_str(QWEN_REPLAY).unwrap(),
    ];
    for report in &reports {
        replay::validate_report(report, &runs).unwrap();
    }
    let mut request = json!({
        "version": 1, "source_id": "journal-faq", "kind": "single", "min_conversations": 2, "limit": 1,
        "selection": {"scope": {"kind": "user", "organization_id": "journal-north", "user_id": "support-17"}},
        "compare": {"after": "2026-11-20T15:00:00+01:00"},
    });
    let list = read(request.clone(), &runs, &reports);
    assert_eq!(list["total_groups"], 1);
    let group = &list["groups"][0];
    assert_eq!(group["primary"]["lower"], 2);
    assert_eq!(group["primary"]["measured_conversations"], 2);
    assert_eq!(group["primary"]["verified_baselines"], 2);
    assert_eq!(group["comparison"]["lower"], 1);
    assert!(group["setup"]["edits"][0]
        .get("replacement_excerpt")
        .is_none());

    request["group"] = group["key"].clone();
    let evidence = read(request.clone(), &runs, &reports);
    assert_eq!(evidence["primary"]["next_offset"], 1);
    let first = &evidence["primary"]["rows"][0];
    assert_eq!(first["location"]["run_id"], "qwen-journal-returns");
    assert_eq!(first["baseline_verified"], true);
    assert_eq!(first["started_at_unix_nanos"], "1795096800000000000");
    assert!(first["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&replay=qwen%2Dscreened%2Djournal%2Dreturns&variant=return%2Dfaq"));
    assert_eq!(
        evidence["comparison"]["rows"][0]["location"]["run_id"],
        "qwen-journal-policy"
    );
    request["offset"] = json!(1);
    let next = read(request.clone(), &runs, &reports);
    assert_eq!(
        next["primary"]["rows"][0],
        evidence["comparison"]["rows"][0]
    );
    assert_eq!(next["comparison"], evidence["comparison"]);

    let mut both = request.clone();
    both["compare"] = json!({});
    both["offset"] = json!(0);
    both["compare_offset"] = json!(1);
    let independently_paged = read(both, &runs, &reports);
    assert_eq!(independently_paged["primary"]["rows"][0], *first);
    assert_eq!(
        independently_paged["comparison"]["rows"][0],
        next["primary"]["rows"][0]
    );

    let mut thread = request.clone();
    thread["selection"] = json!({"thread": {
        "trace_source": "local-transformers", "conversation_id": "journal-return-support",
        "organization_id": "journal-north", "user_id": "support-17"
    }});
    thread["min_conversations"] = json!(1);
    thread["offset"] = json!(0);
    let selected = read(thread, &runs, &reports);
    assert_eq!(selected["primary"]["total_measurements"], 1);
    assert_eq!(selected["primary"]["rows"][0], *first);

    request["include_excerpts"] = json!(true);
    let request = serde_json::from_value(request).unwrap();
    assert!(query::query(&request, &runs, &reports, false)
        .unwrap_err()
        .contains("enabled excerpts"));
    let excerpts = query::query(&request, &runs, &reports, true).unwrap();
    assert_eq!(
        excerpts["group"]["setup"]["edits"][0]["replacement_excerpt"]["text"],
        "The return period from this source is unavailable."
    );
}

#[test]
fn counts_mixed_and_unavailable_decisions_once_and_respects_supplied_runs() {
    let runs: Vec<Run> = serde_json::from_str(QWEN_EVIDENCE).unwrap();
    let original: replay::Replay = serde_json::from_str(QWEN_REPLAY).unwrap();
    let mut case: replay::Case = serde_json::from_str(QWEN_CASE).unwrap();
    case.id = "unchanged-faq-test".into();
    let unchanged = replay::execute(&case, &runs, |_| Some(original.recorded.clone())).unwrap();
    case.id = "failed-faq-test".into();
    case.location.run_id = runs[2].id.clone();
    case.request =
        serde_json::from_str(&runs[2].calls[0].recorded_request.as_ref().unwrap().body).unwrap();
    let failed = replay::execute(&case, &runs, |_| None).unwrap();
    let request = json!({
        "version": 1, "source_id": "journal-faq", "kind": "single",
        "selection": {"scope": {"kind": "organization", "organization_id": "journal-north"}},
        "compare": {"scope": {"kind": "organization", "organization_id": "journal-south"}},
    });
    let reports = [original, unchanged, failed];
    let actual = read(request.clone(), &runs, &reports);
    let group = &actual["groups"][0];
    assert_eq!(group["primary"]["measurements"], 2);
    assert_eq!(group["primary"]["decisions"], 1);
    assert_eq!(group["primary"]["mixed"], 1);
    assert_eq!(group["primary"]["min_difference_pp"].as_f64(), Some(-100.0));
    assert_eq!(group["primary"]["max_difference_pp"].as_f64(), Some(0.0));
    assert_eq!(group["comparison"]["measured_conversations"], 0);
    assert_eq!(group["comparison"]["unavailable"], 1);
    let restricted = read(request, &runs[..2], &reports);
    assert_eq!(restricted["groups"][0]["key"], group["key"]);
    assert!(restricted["groups"][0]["comparison"].is_null());
}

#[test]
fn keeps_target_minus_control_distinct_and_links_to_the_exact_comparison() {
    let mut runs: Vec<Run> =
        serde_json::from_str(include_str!("../examples/local-mlx/evidence.json")).unwrap();
    let reports =
        [serde_json::from_str(include_str!("../examples/local-mlx/report.json")).unwrap()];
    let control = runs[0].calls[0]
        .context
        .iter_mut()
        .find(|item| item.id == "store-name")
        .unwrap();
    control.source_id = Some("store-name".into());
    replay::validate_report(&reports[0], &runs).unwrap();
    let mut request = json!({"version": 1, "kind": "control", "source_id": "store-name"});
    let list = read(request.clone(), &runs, &reports);
    assert_eq!(list["total_groups"], 1);
    let group = &list["groups"][0];
    assert_eq!(group["setup"]["kind"], "control");
    assert_eq!(group["setup"]["edits"][1]["source_id"], "store-name");
    request["group"] = group["key"].clone();
    let evidence = read(request, &runs, &reports);
    let row = &evidence["primary"]["rows"][0];
    assert_eq!(row["difference_pp"].as_f64(), Some(-100.0));
    assert_eq!(row["control"]["matched"], 3);
    assert!(row["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&control=store%2Dname"));
}
