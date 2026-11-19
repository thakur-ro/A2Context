use nexusweave::{
    activation::{patterns, Report},
    Run,
};
use serde_json::Value;

const REPORT: &str = include_str!("../examples/local-mlx/activation-report.json");
const EVIDENCE: &str = include_str!("../examples/local-mlx/evidence.json");
const TIMELINE: &str = include_str!("../examples/local-mlx-timeline/activation-report.json");
const TIMELINE_EVIDENCE: &str = include_str!("../examples/local-mlx-timeline/evidence.json");
const COHORT: &str = include_str!("../examples/local-mlx-timeline/cohort/activation-reports.json");
const COHORT_EVIDENCE: &str = include_str!("../examples/local-mlx-timeline/cohort/evidence.json");
const ROUTING: &str = include_str!("../examples/local-mlx-routing/activation-report.json");
const ROUTING_EVIDENCE: &str = include_str!("../examples/local-mlx-routing/evidence.json");

#[test]
fn visibility_requires_a_reproduced_tool_name_and_arguments() {
    let runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-routing/evidence.json"
    ))
    .unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-routing/report.json"
    ))
    .unwrap();
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(original.clone()).unwrap();
    for proposal in [
        Value::Null,
        serde_json::json!({"name":"price","arguments":{"product_id":42}}),
        serde_json::json!({"name":"stock","arguments":{"product_id":"42"}}),
    ] {
        let mut invalid = original.clone();
        invalid["baseline_tool_call"] = proposal;
        assert!(validate(invalid).unwrap_err().contains("recorded decision"));
    }
}

#[test]
fn visibility_requires_an_exact_captured_object_at_the_reported_request_pointer() {
    let runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-routing/evidence.json"
    ))
    .unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-routing/report.json"
    ))
    .unwrap();
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(original.clone()).unwrap();

    for pointer in [
        Value::Null,
        "/tools/1".into(),
        "/tools/2".into(),
        "/messages/1/content".into(),
    ] {
        let mut invalid = original.clone();
        invalid["sources"][5]["request_pointer"] = pointer;
        assert!(validate(invalid).unwrap_err().contains("recorded request"));
    }
    let mut invalid = original;
    invalid["sources"][1]["request_pointer"] = "/tools/0".into();
    assert!(validate(invalid).unwrap_err().contains("recorded request"));
}

#[test]
fn visibility_pairs_require_two_measured_disjoint_sources_and_one_score_per_token() {
    let runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/paired-visibility-evidence.json"
    ))
    .unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/paired-visibility-transformers.json"
    ))
    .unwrap();
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(original.clone()).unwrap();

    let mut unavailable = original.clone();
    unavailable["sources"][3] =
        serde_json::json!({"context_id":"refund-policy-copy","unavailable":"no_request_fragment"});
    assert!(validate(unavailable)
        .unwrap_err()
        .contains("individually measured"));

    let mut overlap = original.clone();
    overlap["sources"][3]["token_range"] = overlap["sources"][2]["token_range"].clone();
    assert!(validate(overlap).unwrap_err().contains("non-overlapping"));

    let mut duplicate = original.clone();
    let mut reversed = duplicate["pairs"][0].clone();
    reversed["context_ids"].as_array_mut().unwrap().reverse();
    duplicate["pairs"].as_array_mut().unwrap().push(reversed);
    assert!(validate(duplicate)
        .unwrap_err()
        .contains("once per source pair"));

    let mut missing_token = original;
    missing_token["pairs"][0]["masked_log_probs"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(validate(missing_token)
        .unwrap_err()
        .contains("misaligned token scores"));
}

#[test]
fn visibility_targets_require_unique_keys_and_recorded_token_ranges() {
    let runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-target/evidence.json"
    ))
    .unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-target/report.json"
    ))
    .unwrap();
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(original.clone()).unwrap();
    let target = &original["targets"][0];
    for targets in [
        serde_json::json!([target, target]),
        serde_json::json!([{"key":" ","token_range":[7,9]}]),
        serde_json::json!([{"key":"refund","token_range":[7,7]}]),
        serde_json::json!([{"key":"refund","token_range":[7,99]}]),
    ] {
        let mut invalid = original.clone();
        invalid["targets"] = targets;
        assert!(validate(invalid).unwrap_err().contains("output targets"));
    }
}

#[test]
fn visibility_patterns_keep_changed_prefixes_and_source_versions_separate() {
    use nexusweave::activation::visibility::patterns;
    let mut runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/evidence.json"
    ))
    .unwrap();
    let mut reports: Vec<Value> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/reports.json"
    ))
    .unwrap();
    reports[1]["output_token_ids"][0] = 1.into();
    runs[2].calls[0].context[2].source_version = Some("new-version".into());
    runs[3].calls[0].context[2].source_id = None;
    let reports: Vec<Report> = reports
        .into_iter()
        .map(|report| serde_json::from_value(report).unwrap())
        .collect();
    let spec = serde_json::from_value(
        serde_json::json!({"version":1,"output_token":13,"source_id":"refund-policy"}),
    )
    .unwrap();
    let result = patterns::query(&spec, &runs, &reports).unwrap();
    assert_eq!(result["total_groups"], 3);
    assert_eq!(result["coverage"]["measurements_without_source_id"], 1);
    assert!(result["groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["conversation_count"] == 1));
}

#[test]
fn visibility_pattern_order_keeps_scope_filters_and_stable_group_links() {
    use nexusweave::activation::visibility::patterns;
    use serde_json::json;
    let mut runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/evidence.json"
    ))
    .unwrap();
    let reports: Vec<Report> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/reports.json"
    ))
    .unwrap();
    for run in &mut runs {
        if run.organization_id.as_deref() == Some("retail-north") {
            for item in &mut run.calls[0].context {
                if item.source_id.as_deref() == Some("returns-policy") {
                    item.source_id = None;
                }
            }
        }
    }
    let ask = |args| patterns::query(&serde_json::from_value(args).unwrap(), &runs, &reports);
    let mut args = json!({"version":1,"output_token":13,"limit":1});
    let recurring = ask(args.clone()).unwrap();
    assert_eq!(recurring["order"], "recurrence");
    assert_eq!(recurring["groups"][0]["conversation_count"], 4);
    assert_eq!(
        recurring["groups"][0]["comparison"]["sources"][0]["source_id"],
        "format-instruction"
    );

    args["order"] = "magnitude".into();
    let magnitude = ask(args.clone()).unwrap();
    assert_eq!(magnitude["order"], "magnitude");
    let strongest = &magnitude["groups"][0];
    assert_eq!(
        strongest["comparison"]["sources"][0]["source_id"],
        "returns-policy"
    );
    assert_eq!(strongest["conversation_count"], 1);
    args["offset"] = magnitude["next_offset"].clone();
    assert_eq!(
        ask(args.clone()).unwrap()["groups"][0]["key"],
        recurring["groups"][0]["key"]
    );

    args["offset"] = 0.into();
    args["scope"] = json!({"kind":"organization","organization_id":"retail-north"});
    args["compare"] = json!({"scope":{"kind":"organization","organization_id":"retail-south"}});
    let comparison = ask(args.clone()).unwrap();
    let first = &comparison["groups"][0];
    assert_eq!(first["key"], strongest["key"]);
    assert!(first["scope_a"].is_null());
    assert_eq!(first["scope_b"]["conversation_count"], 1);
    args["min_conversations"] = 2.into();
    assert_eq!(
        ask(args.clone()).unwrap()["groups"][0]["key"],
        recurring["groups"][0]["key"]
    );
    args["min_conversations"] = Value::Null;
    args["group"] = strongest["key"].clone();
    args["order"] = "recurrence".into();
    assert_eq!(
        ask(args).unwrap()["scope_b"]["occurrences"][0]["conversation_id"],
        "south-1"
    );
}

#[test]
fn visibility_timeline_aligns_utc_days_and_keeps_undated_evidence() {
    use nexusweave::activation::visibility::patterns;
    use serde_json::json;
    let mut runs: Vec<Run> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/evidence.json"
    ))
    .unwrap();
    runs[0].calls[0].started_at = Some("2026-11-01T23:30:00-02:00".into());
    runs[1].calls[0].started_at = Some("2026-11-02T00:10:00Z".into());
    runs[2].calls[0].started_at = Some("2026-11-01T17:00:00Z".into());
    runs[3].calls[0].started_at = None;
    let reports: Vec<Report> = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/reports.json"
    ))
    .unwrap();
    let mut args = json!({"version":1,"output_token":13,"kind":"pair","source_id":"refund-policy-copy","min_conversations":2});
    let ask = |args| patterns::query(&serde_json::from_value(args).unwrap(), &runs, &reports);
    let groups = ask(args.clone()).unwrap();
    args["group"] = groups["groups"][0]["key"].clone();
    args["timeline"] = json!({"unit":"day"});
    let page = ask(args.clone()).unwrap();
    assert_eq!(page["total_periods"], 3);
    assert_eq!(page["rows"][0]["period"], "2026-11-01");
    assert_eq!(page["rows"][1]["period"], "2026-11-02");
    assert_eq!(page["rows"][1]["scope_a"]["conversation_count"], 2);
    assert_eq!(page["rows"][2]["period"], "undated");
    assert_eq!(page["rows"][2]["scope_a"]["decision_count"], 1);
    args["timeline"]["period"] = "undated".into();
    let undated = ask(args.clone()).unwrap();
    assert_eq!(undated["total_occurrences"], 1);
    assert_eq!(undated["occurrences"][0]["conversation_id"], "south-1");
    assert!(undated["occurrences"][0]["started_at"].is_null());
    args["timeline"] = json!({"unit":"month"});
    args["after"] = "2026-11-01T00:00:00Z".into();
    let page = ask(args).unwrap();
    assert_eq!(page["total_periods"], 1);
    assert_eq!(page["rows"][0]["period"], "2026-11-01");
    assert_eq!(page["rows"][0]["scope_a"]["measurement_count"], 3);
}

#[test]
fn visibility_requires_the_original_request_and_full_source_coverage() {
    let runs: Vec<Run> =
        serde_json::from_str(include_str!("../examples/runtime-comparison/evidence.json")).unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/source-visibility-transformers.json"
    ))
    .unwrap();
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(original.clone()).unwrap();
    let mut wrong_request = original.clone();
    wrong_request["request_sha256"] = "0".repeat(64).into();
    assert!(validate(wrong_request)
        .unwrap_err()
        .contains("request digest"));
    let mut missing = original.clone();
    missing["sources"].as_array_mut().unwrap().pop();
    assert!(validate(missing).unwrap_err().contains("coverage"));
    let mut token_outside_source_space = original;
    token_outside_source_space["sources"][0]["token_range"][1] =
        token_outside_source_space["prompt_token_count"].clone();
    assert!(validate(token_outside_source_space)
        .unwrap_err()
        .contains("invalid text, positions, or scores"));
}

#[test]
fn time_window_filters_group_totals_and_drill_down_together() {
    let mut runs: Vec<Run> = serde_json::from_str(COHORT_EVIDENCE).unwrap();
    let reports: Vec<Report> = serde_json::from_str(COHORT).unwrap();
    runs[0].calls[0].started_at = Some("2026-11-19T05:00:00-05:00".into());
    runs[1].calls[0].started_at = Some("2026-11-19T12:00:00Z".into());
    runs[2].calls[0].started_at = None;
    runs[3].calls[0].started_at = Some("2026-11-19T11:00:00Z".into());
    let mut spec = serde_json::json!({
        "version": 1, "block": 0, "source_id": "returns-policy", "checkpoint": "Refund timing",
        "scope": { "kind": "user", "organization_id": "demo-retail-north", "user_id": "customer-01" }
    });
    let query = |value: &Value| {
        let request = serde_json::from_value(value.clone()).unwrap();
        patterns::query::query(&request, &runs, &reports, false)
    };
    let all = query(&spec).unwrap();
    assert_eq!(all["groups"][0]["conversation_count"], 3);
    spec["after"] = "2026-11-19T10:00:00Z".into();
    spec["before"] = "2026-11-19T12:00:00Z".into();
    let scoped = query(&spec).unwrap();
    let group = &scoped["groups"][0];
    assert_eq!(group["key"], all["groups"][0]["key"]);
    assert_eq!(group["conversation_count"], 1);
    assert_eq!(group["measurement_count"], 1);
    spec["group"] = group["key"].clone();
    let contributing = query(&spec).unwrap();
    assert_eq!(contributing["total_occurrences"], 1);
    assert_eq!(
        contributing["occurrences"][0]["location"]["run_id"],
        runs[0].id
    );
    assert!(contributing["next_offset"].is_null());

    spec["before"] = spec["after"].clone();
    assert!(query(&spec).unwrap_err().contains("must precede"));
}

#[test]
fn attaches_measured_report_only_to_matching_captured_sources_and_decision() {
    let mut runs: Vec<Run> = serde_json::from_str(EVIDENCE).unwrap();
    let report: Report = serde_json::from_str(REPORT).unwrap();
    report.validate(&runs).unwrap();

    runs[0].calls[0].decisions[0].outcome = 60.into();
    assert!(report
        .validate(&runs)
        .unwrap_err()
        .contains("recorded decision"));
    runs[0].calls[0].decisions[0].outcome = 30.into();
    runs[0].calls[0].context[0].text = Some("Changed captured policy".into());
    assert!(report
        .validate(&runs)
        .unwrap_err()
        .contains("source text differs"));
}

#[test]
fn refuses_a_sweep_at_another_sources_token_positions() {
    let runs: Vec<Run> = serde_json::from_str(EVIDENCE).unwrap();
    let mut report: Value = serde_json::from_str(REPORT).unwrap();
    report["sweeps"][0]["token_range"] = report["sweeps"][1]["token_range"].clone();
    let report: Report = serde_json::from_value(report).unwrap();
    assert!(report
        .validate(&runs)
        .unwrap_err()
        .contains("invalid positions"));
}

#[test]
fn activation_timeline_checks_output_alignment_and_reports_missing_source_measurements() {
    let runs: Vec<Run> = serde_json::from_str(TIMELINE_EVIDENCE).unwrap();
    let original: Value = serde_json::from_str(TIMELINE).unwrap();
    let report: Report = serde_json::from_value(original.clone()).unwrap();
    report.validate(&runs).unwrap();
    assert_eq!(serde_json::to_value(report).unwrap(), original);

    for (key, replacement) in [
        ("output_token", 3.into()),
        ("answer_prefix", "Window: 60 days\nRefund: ".into()),
        ("choice_token_ids", serde_json::json!([42, 43])),
    ] {
        let mut value = original.clone();
        value["checkpoints"][1][key] = replacement;
        let report: Report = serde_json::from_value(value).unwrap();
        assert!(report
            .validate(&runs)
            .unwrap_err()
            .contains("does not match the baseline answer"));
    }

    let mut value = original;
    value["checkpoints"][1]["sweeps"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let report: Report = serde_json::from_value(value).unwrap();
    assert!(report
        .validate(&runs)
        .unwrap_err()
        .contains("must sweep the edited tokens of each source"));
}

#[test]
fn activation_patterns_match_the_checkpoint_prefix_and_model_revision() {
    let runs: Vec<Run> = serde_json::from_str(COHORT_EVIDENCE).unwrap();
    let mut reports: Vec<Report> = serde_json::from_str(COHORT).unwrap();
    for report in &reports {
        report.validate(&runs).unwrap();
    }
    let mut revised = serde_json::to_value(&reports[0]).unwrap();
    revised["model_revision"] = "different-revision".into();
    revised["id"] = "calibration-run".into();
    reports.push(serde_json::from_value(revised).unwrap());
    let run_refs: Vec<_> = runs.iter().collect();
    let groups = serde_json::to_value(patterns::group(&run_refs, &reports)).unwrap();
    let select = |checkpoint, revision_changed| {
        let mut counts = groups
            .as_array()
            .unwrap()
            .iter()
            .filter(|group| {
                group["source_id"] == "returns-policy"
                    && group["checkpoint"]["label"] == checkpoint
                    && (group["model_revision"] == "different-revision") == revision_changed
            })
            .map(|group| group["occurrences"].as_array().unwrap().len())
            .collect::<Vec<_>>();
        counts.sort_unstable();
        counts
    };
    assert_eq!(select("Return window", false), [6]);
    assert_eq!(select("Refund timing", false), [2, 4]);
    assert_eq!(select("Refund timing", true), [1]);
}

#[test]
fn attaches_tool_measurements_to_the_exact_proposed_name_and_arguments() {
    let runs: Vec<Run> = serde_json::from_str(ROUTING_EVIDENCE).unwrap();
    let original: Value = serde_json::from_str(ROUTING).unwrap();
    let report: Report = serde_json::from_value(original.clone()).unwrap();
    report.validate(&runs).unwrap();
    let group = serde_json::to_value(patterns::group(&[&runs[0]], &[report])).unwrap();
    let groups = group.as_array().unwrap();
    assert_eq!(groups.len(), 6);
    assert!(groups
        .iter()
        .all(|group| group["decision_kind"] == "tool_call" && group["decision_label"] == "stock"));

    for (field, replacement) in [
        ("name", "price".into()),
        ("arguments", serde_json::json!({"product_id": 73})),
    ] {
        let mut value = original.clone();
        value["tool_calls"]["baseline"][field] = replacement;
        let report: Report = serde_json::from_value(value).unwrap();
        assert!(report
            .validate(&runs)
            .unwrap_err()
            .contains("did not reproduce"));
    }
    let mut value = original;
    value["tool_calls"]["edited"]
        .as_object_mut()
        .unwrap()
        .remove("store-name");
    let report: Report = serde_json::from_value(value).unwrap();
    assert!(report
        .validate(&runs)
        .unwrap_err()
        .contains("each source edit"));
}

#[test]
fn accepts_both_runtime_reports_without_merging_their_measurements() {
    let runs: Vec<Run> =
        serde_json::from_str(include_str!("../examples/runtime-comparison/evidence.json")).unwrap();
    let reports: Vec<Report> = [
        include_str!("../examples/runtime-comparison/activation-report-mlx.json"),
        include_str!("../examples/runtime-comparison/activation-report-transformers.json"),
    ]
    .into_iter()
    .map(|json| serde_json::from_str(json).unwrap())
    .collect();
    for report in &reports {
        report.validate(&runs).unwrap();
    }
    let refs: Vec<_> = runs.iter().collect();
    let groups = patterns::group(&refs, &reports);
    assert_eq!(groups.len(), 12);
    assert!(groups.iter().all(|group| group.occurrences.len() == 1));
    let groups = serde_json::to_value(groups).unwrap();
    let windows: Vec<_> = groups
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| {
            group["source_id"] == "returns-policy"
                && group["checkpoint"]["label"] == "Return window"
        })
        .collect();
    assert_eq!(windows.len(), 2);
    assert_ne!(windows[0]["runtime"], windows[1]["runtime"]);
    assert!(windows.iter().all(|group| {
        group["occurrences"][0]["patched_margins"][0]
            .as_f64()
            .unwrap()
            < 0.0
    }));
}

#[test]
fn requires_output_scores_for_every_token_with_matching_text_and_controls() {
    let runs: Vec<Run> = serde_json::from_str(ROUTING_EVIDENCE).unwrap();
    let value: Value = serde_json::from_str(include_str!(
        "../examples/local-mlx-routing/output-token-report.json"
    ))
    .unwrap();
    let report: Report = serde_json::from_value(value.clone()).unwrap();
    report.validate(&runs).unwrap();
    let mut only_output = value.clone();
    only_output["readout"] = "output_token_log_probability".into();
    only_output["checkpoints"] = serde_json::json!([]);
    let report: Report = serde_json::from_value(only_output).unwrap();
    report.validate(&runs).unwrap();
    assert!(!report.has_checkpoints());

    for (path, replacement, message) in [
        (
            "/output_scores/fragments/7",
            "73".into(),
            "align with every",
        ),
        (
            "/output_scores/sources/product-id/block_log_probs/0/7",
            0.5.into(),
            "one log probability",
        ),
        (
            "/output_scores/checks/full_donor_log_probs/7",
            (-100.0).into(),
            "instrumentation controls",
        ),
    ] {
        let mut invalid = value.clone();
        *invalid.pointer_mut(path).unwrap() = replacement;
        let report: Report = serde_json::from_value(invalid).unwrap();
        assert!(report.validate(&runs).unwrap_err().contains(message));
    }
}

#[test]
fn generation_branches_keep_the_recorded_prefix_and_verify_patch_receipts() {
    let runs: Vec<Run> =
        serde_json::from_str(include_str!("../examples/runtime-comparison/evidence.json")).unwrap();
    let original: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/generation-branches-transformers.json"
    ))
    .unwrap();
    let report: Report = serde_json::from_value(original.clone()).unwrap();
    report.validate(&runs).unwrap();

    for (path, replacement, error) in [
        (
            "/generation_branches/0/patched/token_ids/0",
            0.into(),
            "fixed prefix",
        ),
        (
            "/generation_branches/0/clean/text",
            "Different clean output".into(),
            "clean swap",
        ),
        (
            "/generation_branches/0/patched_steps",
            1.into(),
            "decoding steps",
        ),
        (
            "/generation_branches/0/max_new_tokens",
            1.into(),
            "clean swap",
        ),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(path).unwrap() = replacement;
        let report: Report = serde_json::from_value(value).unwrap();
        assert!(report.validate(&runs).unwrap_err().contains(error));
    }
}
