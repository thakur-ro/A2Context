use nexusweave::{
    inspect, otlp, replay, replay_effects, Availability, Evidence, RecordedRequest,
    RequestFragment, Run, TextRange,
};
use replay::{CaptureAvailability, MatchRule, Operation, ReplayInterval, SamplingDesign};
use replay_effects::{ComparisonTarget, OccurrenceTarget};
use serde_json::{json, Value};

fn inputs() -> (Vec<Run>, replay::Case) {
    (
        serde_json::from_str(include_str!("../fixtures/support.json")).unwrap(),
        serde_json::from_str(include_str!("../fixtures/replay-case.json")).unwrap(),
    )
}

fn runner(request: &Value) -> replay::Choice {
    let days = if request["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("30-day")
    {
        30
    } else {
        60
    };
    replay::Choice {
        kind: "tool_call".into(),
        label: "check_return_eligibility".into(),
        outcome: json!({"window_days":days}),
    }
}

#[test]
fn capture_inventory_distinguishes_exact_fields_from_unavailable_evidence() {
    let (mut runs, case) = inputs();
    let call = &mut runs[0].calls[0];
    let mut request = case.request;
    request["copy"] = request["messages"][0]["content"].clone();
    request["mock/~record"] = json!({"version": 9223372036854775807u64, "active": true});
    request["different"] = json!({"version": 9223372036854775806u64, "active": true});
    request["scalar"] = json!(7);
    call.recorded_request = Some(RecordedRequest {
        body: request.to_string(),
        evidence: Evidence {
            span_id: None,
            field: "inference.request".into(),
            message_index: None,
            part_index: None,
        },
    });
    for (id, value, reference, availability) in [
        (
            "object",
            Some(r#"{"active":true,"version":9223372036854775807}"#.into()),
            None,
            Availability::Included,
        ),
        (
            "request-root",
            Some(request.to_string()),
            None,
            Availability::Included,
        ),
        ("scalar", Some("7".into()), None, Availability::Included),
        (
            "ref",
            None,
            Some("context://skipped".into()),
            Availability::Unknown,
        ),
        ("masked", None, None, Availability::Redacted),
        ("missing", None, None, Availability::Included),
    ] {
        let mut item = call.context[0].clone();
        item.id = id.into();
        item.text = value;
        item.rule = None;
        item.content_reference = reference;
        item.availability = availability;
        call.context.push(item);
    }
    let inspection = inspect(&runs).unwrap();
    let call = inspection
        .capture_profiles
        .iter()
        .find(|item| item.run_id == "return-eligibility" && item.call_id == "check")
        .unwrap();
    assert_eq!(call.request_evidence.unwrap().field, "inference.request");
    let entry = |id| {
        let item = call
            .items
            .iter()
            .find(|item| item.context_id == id)
            .unwrap();
        (item.capture, item.matching_fields)
    };
    assert_eq!(
        entry("returns-policy"),
        (CaptureAvailability::Available, Some(2))
    );
    assert_eq!(entry("object"), (CaptureAvailability::Available, Some(1)));
    assert_eq!(
        entry("request-root"),
        (CaptureAvailability::Available, Some(0))
    );
    assert_eq!(entry("scalar"), (CaptureAvailability::Available, Some(0)));
    assert_eq!(entry("ref"), (CaptureAvailability::ReferenceOnly, None));
    assert_eq!(entry("masked"), (CaptureAvailability::Redacted, None));
    assert_eq!(entry("missing"), (CaptureAvailability::Unavailable, None));
    let without_request = inspection
        .capture_profiles
        .iter()
        .find(|item| item.run_id == "return-eligibility" && item.call_id == "follow-up")
        .unwrap();
    assert!(without_request.request_evidence.is_none());
    assert!(without_request
        .items
        .iter()
        .all(|item| item.matching_fields.is_none()));
    let fields = replay::request_field_matches(&runs[0].calls[0], &request).unwrap();
    let object = fields
        .iter()
        .find(|item| item.context_id == "object")
        .unwrap();
    assert_eq!(object.fields.len(), 1);
    assert_eq!(object.fields[0].pointer, "/mock~1~0record");
    assert!(matches!(
        object.fields[0].kind,
        replay::RequestFieldKind::Object
    ));
}

#[test]
fn validates_native_json_edits_before_sampling_and_preserves_wide_integers() {
    let (mut runs, mut case) = inputs();
    let context = &mut runs[0].calls[0].context[0];
    context.text = Some(r#"{"version":9223372036854775807,"active":true}"#.into());
    case.request = json!({"record": {"version":9223372036854775807u64,"active":true}});
    case.samples = 1;
    case.interventions.truncate(1);
    let edit = &mut case.interventions[0];
    edit.pointer = "/record".into();
    edit.operation = Operation::ReplaceJson;
    edit.replacement = r#"{"active":true,"version":9223372036854775807}"#.into();
    assert!(replay::validate_case(&case, &runs)
        .unwrap_err()
        .contains("unchanged"));

    case.interventions[0].replacement = r#"{"active":true,"version":9223372036854775806}"#.into();
    replay::validate_case(&case, &runs).unwrap();
    let mut versions = Vec::new();
    replay::execute(&case, &runs, |request| {
        versions.push(request["record"]["version"].as_u64().unwrap());
        None
    })
    .unwrap();
    assert_eq!(versions, [9223372036854775807, 9223372036854775806]);
}

#[test]
fn anchors_replay_to_the_recorded_request_and_rejects_a_changed_baseline_before_sampling() {
    let (mut runs, mut case) = inputs();
    case.request["seed"] = json!(9223372036854775807u64);
    runs[0].calls[0].recorded_request = Some(RecordedRequest {
        body: serde_json::to_string_pretty(&case.request).unwrap(),
        evidence: Evidence {
            span_id: None,
            field: "inference.request".into(),
            message_index: None,
            part_index: None,
        },
    });
    let mut report = replay::execute(&case, &runs, |request| {
        assert_eq!(request["seed"], json!(9223372036854775807u64));
        Some(runner(request))
    })
    .unwrap();
    replay::validate_report(&report, &runs).unwrap();
    report.variants[0].intervention.pointer = "/messages/1/content".into();
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("differs from the recorded context"));
    report.baseline_request_sha256 = "0".repeat(64);
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("baseline differs"));
    case.request["seed"] = json!(123);
    assert!(replay::execute(&case, &runs, |_| panic!(
        "request must be rejected before sampling"
    ))
    .unwrap_err()
    .contains("baseline differs"));
}

#[test]
fn detects_the_planted_driver_while_holding_other_inputs_fixed() {
    let (runs, case) = inputs();
    let mut requests = Vec::new();
    let report = replay::execute(&case, &runs, |request| {
        requests.push(request.clone());
        Some(runner(request))
    })
    .unwrap();
    assert_eq!(requests.len(), 12);
    let mut without_policy = case.request.clone();
    without_policy["messages"][0]["content"] = json!("[context omitted]");
    let mut without_preference = case.request.clone();
    without_preference["messages"][1]["content"] = json!("[context omitted]");
    assert_eq!(
        requests[..3],
        [case.request, without_policy, without_preference]
    );
    assert_eq!(report.baseline_summary.matched, 4);
    assert_eq!(report.variants[0].summary.matched, 0);
    assert_eq!(report.variants[0].difference_pp, Some(-100.0));
    assert_eq!(report.variants[1].difference_pp, Some(0.0));
    replay::validate_report(&report, &runs).unwrap();
    let mut changed = runs;
    changed[0].calls[0].context[0].text = Some("Use a different policy.".into());
    assert!(replay::validate_report(&report, &changed)
        .unwrap_err()
        .contains("different context text"));
}

#[test]
fn independent_sampling_intervals_reflect_trial_count_and_survive_serialization() {
    let (runs, mut case) = inputs();
    case.sampling_design = SamplingDesign::Independent;
    case.samples = 3;
    let mut small = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    assert_eq!(small.variants[0].difference_pp, Some(-100.0));
    assert_eq!(
        small.variants[0].difference_95_interval_pp,
        Some(ReplayInterval {
            lower: -100,
            upper: 11
        })
    );
    assert_eq!(
        small.variants[1].difference_95_interval_pp,
        Some(ReplayInterval {
            lower: -100,
            upper: 100
        })
    );
    let restored: replay::Replay =
        serde_json::from_value(serde_json::to_value(&small).unwrap()).unwrap();
    replay::validate_report(&restored, &runs).unwrap();

    small.variants[0]
        .difference_95_interval_pp
        .as_mut()
        .unwrap()
        .upper = 0;
    assert!(replay::validate_report(&small, &runs)
        .unwrap_err()
        .contains("interval"));

    case.samples = 100;
    let large = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    assert_eq!(
        large.variants[0].difference_95_interval_pp,
        Some(ReplayInterval {
            lower: -100,
            upper: -80
        })
    );
    assert_eq!(
        large.variants[1].difference_95_interval_pp,
        Some(ReplayInterval {
            lower: -20,
            upper: 20
        })
    );

    case.id = "unspecified-sampling".into();
    case.sampling_design = SamplingDesign::Unspecified;
    let unspecified = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    assert!(unspecified.variants[0].difference_95_interval_pp.is_none());
    let reports = [large, unspecified];
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    assert_eq!(groups.len(), 4);
    assert_eq!(
        groups
            .iter()
            .filter(|group| group.occurrences[0].difference_95_interval_pp.is_some())
            .count(),
        2
    );
}

#[test]
fn compares_a_target_with_a_declared_negative_control() {
    let (runs, mut case) = inputs();
    case.samples = 100;
    case.sampling_design = SamplingDesign::Independent;
    case.negative_control_context_id = Some("return-preference".into());
    // The declaration matches the context ID regardless of intervention order.
    case.interventions.reverse();
    let mut report = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    let comparison = report.control_comparison.as_ref().unwrap();
    assert_eq!(comparison.control_context_id, "return-preference");
    assert_eq!(comparison.target_minus_control_pp, Some(-100.0));
    assert_eq!(
        comparison.target_minus_control_95_interval_pp,
        Some(ReplayInterval {
            lower: -100,
            upper: -80
        })
    );
    replay::validate_report(&report, &runs).unwrap();
    report
        .control_comparison
        .as_mut()
        .unwrap()
        .target_minus_control_pp = Some(0.0);
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("negative control comparison"));
    let report = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    let rescored = replay::rescore(report, &runs, "label-only", MatchRule::KindAndLabel).unwrap();
    let comparison = rescored.control_comparison.as_ref().unwrap();
    assert_eq!(comparison.target_minus_control_pp, Some(0.0));
    assert_eq!(comparison.target_minus_control_95_interval_pp, None);
    replay::validate_report(&rescored, &runs).unwrap();
}

#[test]
fn a_negative_control_requires_two_edits_and_scored_trials_in_all_conditions() {
    let (runs, mut case) = inputs();
    case.negative_control_context_id = Some("unknown".into());
    assert!(
        replay::execute(&case, &runs, |_| panic!("sampled an invalid case"))
            .unwrap_err()
            .contains("identify one")
    );
    case.negative_control_context_id = Some("return-preference".into());
    let report = replay::execute(&case, &runs, |request| {
        (request["messages"][1]["content"] != "[context omitted]").then(|| runner(request))
    })
    .unwrap();
    assert_eq!(
        report.control_comparison.unwrap().target_minus_control_pp,
        None
    );
    case.interventions.pop();
    assert!(
        replay::execute(&case, &runs, |_| panic!("sampled an invalid case"))
            .unwrap_err()
            .contains("exactly two interventions")
    );
}

#[test]
fn groups_control_comparisons_across_scopes_and_preserves_edit_roles() {
    let (runs, mut case) = inputs();
    case.negative_control_context_id = Some("return-preference".into());
    let first = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    case.location.run_id = "other-store".into();
    case.interventions.reverse();
    let second = replay::execute(&case, &runs, |request| {
        (request["messages"][1]["content"] != "[context omitted]").then(|| runner(request))
    })
    .unwrap();
    case.negative_control_context_id = Some("returns-policy".into());
    let reversed = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    let reports = [first, second, reversed];
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    let main = groups
        .iter()
        .find(|group| {
            matches!(group.comparison.target,
                ComparisonTarget::Control { target_edit, control_edit }
                if target_edit.source_id == Some("returns-policy")
                    && control_edit.source_id == Some("return-preference")
            )
        })
        .unwrap();
    assert_eq!(main.occurrences.len(), 2);
    assert_eq!(main.occurrences[0].organization_id, Some("demo-retail"));
    assert_eq!(main.occurrences[0].difference_pp, Some(-100.0));
    assert_eq!(main.occurrences[1].organization_id, Some("second-retailer"));
    assert_eq!(main.occurrences[1].difference_pp, None);
    assert!(main.occurrences.iter().all(|item| matches!(
        item.target,
        OccurrenceTarget::Control {
            target_context_id: "returns-policy",
            control_context_id: "return-preference",
            ..
        }
    )));
    let flipped = groups.iter().find(|group| matches!(group.comparison.target,
        ComparisonTarget::Control { target_edit, .. } if target_edit.source_id == Some("return-preference")
    )).unwrap();
    assert_eq!(flipped.occurrences.len(), 1);
    assert_eq!(flipped.occurrences[0].difference_pp, Some(100.0));
    assert!(replay_effects::by_context(&groups)
        .iter()
        .flat_map(|item| &item.effects)
        .all(|reference| matches!(
            groups[reference.group_index].comparison.target,
            ComparisonTarget::Single(_)
        )));
}

#[test]
fn pair_intervals_apply_to_original_scoring_and_usable_contrasts() {
    let (runs, mut case) = inputs();
    case.test_pair = true;
    case.samples = 100;
    case.sampling_design = SamplingDesign::Independent;
    let sample = |request: &Value| {
        let omitted =
            [0, 1].map(|index| request["messages"][index]["content"] == "[context omitted]");
        let days = if omitted == [true, true] { 60 } else { 30 };
        let mut choice = runner(request);
        choice.outcome = json!({"window_days":days});
        Some(choice)
    };
    let report = replay::execute(&case, &runs, sample).unwrap();
    let pair = report.pair.as_ref().unwrap();
    assert_eq!(
        pair.difference_95_interval_pp,
        Some(ReplayInterval {
            lower: -100,
            upper: -80
        })
    );
    assert_eq!(
        pair.interaction_95_interval_pp,
        Some(ReplayInterval {
            lower: -128,
            upper: -72
        })
    );
    let rescored =
        replay::rescore(report, &runs, "rescored-independent", MatchRule::Exact).unwrap();
    assert_eq!(rescored.sampling_design, SamplingDesign::Independent);
    assert_eq!(rescored.pair.as_ref().unwrap().interaction_pp, Some(-100.0));
    assert!(rescored
        .pair
        .as_ref()
        .unwrap()
        .interaction_95_interval_pp
        .is_none());
    assert!(rescored
        .variants
        .iter()
        .all(|variant| variant.difference_95_interval_pp.is_none()));
    replay::validate_report(&rescored, &runs).unwrap();

    let incomplete = replay::execute(&case, &runs, |request| {
        if request["messages"][0]["content"] == "[context omitted]"
            && request["messages"][1]["content"] != "[context omitted]"
        {
            return None;
        }
        sample(request)
    })
    .unwrap();
    assert!(incomplete.variants[0].difference_95_interval_pp.is_none());
    assert!(incomplete
        .pair
        .as_ref()
        .unwrap()
        .difference_95_interval_pp
        .is_some());
    assert!(incomplete
        .pair
        .as_ref()
        .unwrap()
        .interaction_95_interval_pp
        .is_none());
}

#[test]
fn withholds_effect_when_baseline_does_not_reproduce_or_samples_fail() {
    let (runs, case) = inputs();
    let report = replay::execute(&case, &runs, |_| {
        Some(replay::Choice {
            kind: "tool_call".into(),
            label: "other_choice".into(),
            outcome: Value::Null,
        })
    })
    .unwrap();
    assert_eq!(report.baseline_summary.matched, 0);
    assert!(report
        .variants
        .iter()
        .all(|variant| variant.difference_pp.is_none()));

    let mut report = replay::execute(&case, &runs, |request| {
        if request["messages"][0]["content"] == "[context omitted]" {
            None
        } else {
            Some(runner(request))
        }
    })
    .unwrap();
    assert_eq!(report.variants[0].summary.completed, 0);
    assert_eq!(report.variants[0].difference_pp, None);
    assert_eq!(report.variants[1].difference_pp, Some(0.0));
    report.variants[1].summary.matched = 3;
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("replay scores"));
}

#[test]
fn edits_only_declared_utf8_fragments_and_keeps_joint_edits_in_original_coordinates() {
    let (mut runs, mut case) = inputs();
    let call = &mut runs[0].calls[0];
    let policy = call.context[0].text.clone().unwrap();
    let preference = call.context[1].text.clone().unwrap();
    let prefix = "Préface 🧭\n";
    let separator = "\n---\n";
    let prompt = |left: &str, right: &str| format!("{prefix}{left}{separator}{right}\nFin.");
    let contents = prompt(&policy, &preference);
    let pointer = "/messages/0/content";
    case.request = json!({"messages":[{"role":"system","content":contents}], "archive":policy});
    call.recorded_request = Some(RecordedRequest {
        body: case.request.to_string(),
        evidence: call.context[0].evidence.clone(),
    });
    for (index, (start, text)) in [
        (prefix.len(), policy.as_str()),
        (
            prefix.len() + policy.len() + separator.len(),
            preference.as_str(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let range = TextRange {
            start_byte: start,
            end_byte: start + text.len(),
        };
        call.context[index].request_fragment = Some(RequestFragment {
            pointer: pointer.into(),
            text_range: range,
        });
        let intervention = &mut case.interventions[index];
        intervention.pointer = pointer.into();
        intervention.operation = Operation::ReplaceTextFragment;
        intervention.text_range = Some(range);
        intervention.replacement =
            ["", "A longer replacement 🌿 for the second source"][index].into();
    }
    let plan = replay::request_field_matches(call, &case.request).unwrap();
    assert_eq!(plan[0].fields.len(), 1);
    assert_eq!(plan[0].fields[0].pointer, pointer);
    assert_eq!(
        plan[0].fields[0].text_range,
        case.interventions[0].text_range
    );
    case.samples = 1;
    case.test_pair = true;
    case.negative_control_context_id = Some("return-preference".into());
    let mut contents = Vec::new();
    let mut report = replay::execute(&case, &runs, |request| {
        assert_eq!(request["archive"], policy);
        contents.push(
            request
                .pointer(pointer)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        );
        Some(runner(request))
    })
    .unwrap();
    let new_preference = &case.interventions[1].replacement;
    assert_eq!(
        contents,
        [
            prompt(&policy, &preference),
            prompt("", &preference),
            prompt(&policy, new_preference),
            prompt("", new_preference),
        ]
    );
    assert_eq!(report.pair.as_ref().unwrap().difference_pp, Some(-100.0));
    assert_eq!(
        report
            .control_comparison
            .as_ref()
            .unwrap()
            .target_minus_control_pp,
        Some(-100.0)
    );
    replay::validate_report(&report, &runs).unwrap();
    report.variants[0]
        .intervention
        .text_range
        .as_mut()
        .unwrap()
        .start_byte += 1;
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("declared fragment"));

    // Even an identical copy elsewhere in the request cannot replace the anchored source.
    case.interventions[0].pointer = "/archive".into();
    case.interventions[0].operation = Operation::ReplaceText;
    case.interventions[0].text_range = None;
    assert!(
        replay::execute(&case, &runs, |_| panic!("sampled a different input"))
            .unwrap_err()
            .contains("declared fragment")
    );
}

#[test]
fn rejects_overlapping_ranges_and_ranges_inside_utf8_code_points() {
    let (mut runs, mut case) = inputs();
    let call = &mut runs[0].calls[0];
    case.request = json!({"text":"éabc", "copy":"ab"});
    for (index, (text, start, end)) in [("ab", 2, 4), ("bc", 3, 5)].into_iter().enumerate() {
        let range = TextRange {
            start_byte: start,
            end_byte: end,
        };
        call.context[index].text = Some(text.into());
        call.context[index].request_fragment = Some(RequestFragment {
            pointer: "/text".into(),
            text_range: range,
        });
        let edit = &mut case.interventions[index];
        edit.pointer = "/text".into();
        edit.operation = Operation::ReplaceTextFragment;
        edit.text_range = Some(range);
    }
    case.test_pair = true;
    assert!(replay::validate_case(&case, &runs)
        .unwrap_err()
        .contains("non-overlapping"));
    case.test_pair = false;
    case.negative_control_context_id = Some("return-preference".into());
    assert!(replay::validate_case(&case, &runs)
        .unwrap_err()
        .contains("non-overlapping"));
    let call = &mut runs[0].calls[0];
    let range = TextRange {
        start_byte: 1,
        end_byte: 4,
    };
    call.context[0]
        .request_fragment
        .as_mut()
        .unwrap()
        .text_range = range;
    case.interventions[0].text_range = Some(range);
    let matched = replay::request_field_matches(call, &case.request).unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].context_id, "return-preference");
    assert!(replay::validate_case(&case, &runs)
        .unwrap_err()
        .contains("differs from the recorded context"));
}

#[test]
fn checks_recorded_evidence_before_invoking_the_runner() {
    let (runs, mut case) = inputs();
    case.interventions[0].pointer = "/messages/1/content".into();
    let result = replay::execute(&case, &runs, |_| panic!("runner must not be called"));
    assert!(result
        .unwrap_err()
        .contains("differs from the recorded context"));
}

#[test]
fn groups_replay_effects_with_their_original_scope_and_does_not_pool_trials() {
    let (mut runs, mut case) = inputs();
    runs[0].calls[0].started_at = Some("2026-01-01T00:00:00Z".into());
    runs[0].calls[1].started_at = Some("2026-01-01T00:00:01Z".into());
    let mut reports = Vec::new();
    for (run_id, call_id) in [
        ("return-eligibility", "check"),
        ("return-eligibility", "follow-up"),
        ("damaged-item", "check"),
        ("other-store", "check"),
    ] {
        case.location.run_id = run_id.into();
        case.location.call_id = call_id.into();
        let report = replay::execute(&case, &runs, |request| {
            (run_id != "other-store" || request["messages"][0]["content"] != "[context omitted]")
                .then(|| runner(request))
        })
        .unwrap();
        reports.push(report);
    }
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    let policy = groups
        .iter()
        .find(|group| {
            matches!(&group.comparison.target, ComparisonTarget::Single(change)
            if change.source_id == Some("returns-policy"))
        })
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(policy.occurrences.len(), 4);
    assert!(policy
        .occurrences
        .iter()
        .all(|item| item.user_id == Some("user-042")));
    assert_eq!(
        policy.occurrences[0].conversation_id,
        Some("return-eligibility")
    );
    assert_eq!(policy.occurrences[1].location.call_id, "follow-up");
    assert_eq!(policy.occurrences[0].turn_id, policy.occurrences[1].turn_id);
    assert_eq!(
        policy.occurrences[1].started_at,
        Some("2026-01-01T00:00:01Z")
    );
    assert_eq!(policy.occurrences[2].conversation_id, Some("damaged-item"));
    assert_eq!(policy.occurrences[2].difference_pp, Some(-100.0));
    assert_eq!(policy.occurrences[2].baseline.matched, 4);
    assert_eq!(
        policy.occurrences[3].organization_id,
        Some("second-retailer")
    );
    assert_eq!(policy.occurrences[3].variant.completed, 0);
    assert_eq!(policy.occurrences[3].difference_pp, None);
}

#[test]
fn splits_effects_for_changed_text_runner_or_replacement_even_with_the_same_source_version() {
    let (mut runs, mut case) = inputs();
    case.interventions.truncate(1);
    let mut reports = vec![replay::execute(&case, &runs, |request| Some(runner(request))).unwrap()];

    case.runner_id = "Different runner".into();
    reports.push(replay::execute(&case, &runs, |request| Some(runner(request))).unwrap());
    case.runner_id = reports[0].runner_id.clone();
    case.interventions[0].replacement = "No returns are offered.".into();
    reports.push(replay::execute(&case, &runs, |request| Some(runner(request))).unwrap());
    case.interventions[0].replacement = reports[0].variants[0].intervention.replacement.clone();

    let text = "Standard returns use a 30-day window.";
    case.location.run_id = "damaged-item".into();
    case.request["messages"][0]["content"] = json!(text);
    let run = runs
        .iter_mut()
        .find(|run| run.id == case.location.run_id)
        .unwrap();
    run.calls[0].context[0].text = Some(text.into());
    reports.push(replay::execute(&case, &runs, |request| Some(runner(request))).unwrap());

    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    assert_eq!(groups.len(), 4);
    assert!(groups.iter().all(|group| group.occurrences.len() == 1
        && matches!(&group.comparison.target, ComparisonTarget::Single(change)
            if change.source_version == Some("v12"))));
}

#[test]
fn tests_all_four_conditions_and_finds_a_driver_shared_by_two_items() {
    let (runs, mut case) = inputs();
    case.test_pair = true;
    let mut sent = Vec::new();
    let mut report = replay::execute(&case, &runs, |request| {
        let kept = [0, 1].map(|index| request["messages"][index]["content"] != "[context omitted]");
        sent.push(kept);
        let days = if kept.contains(&true) { 30 } else { 60 };
        let mut decision = runner(request);
        decision.outcome = json!({"window_days":days});
        Some(decision)
    })
    .unwrap();
    assert_eq!(sent.len(), 16);
    assert_eq!(
        &sent[..4],
        &[[true, true], [false, true], [true, false], [false, false]]
    );
    assert_eq!(report.baseline_summary.matched, 4);
    assert!(report
        .variants
        .iter()
        .all(|variant| variant.difference_pp == Some(0.0)));
    let pair = report.pair.as_ref().unwrap();
    assert_eq!(pair.summary.matched, 0);
    assert_eq!(pair.difference_pp, Some(-100.0));
    assert_eq!(pair.interaction_pp, Some(-100.0));
    replay::validate_report(&report, &runs).unwrap();
    report.pair.as_mut().unwrap().interaction_pp = Some(0.0);
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("pair replay scores"));
}

#[test]
fn rejects_invalid_pair_designs_before_replay_and_leaves_an_incomplete_interaction_unknown() {
    let (mut runs, mut case) = inputs();
    case.test_pair = true;
    let report = replay::execute(&case, &runs, |request| {
        let first = request["messages"][0]["content"] == "[context omitted]";
        let second = request["messages"][1]["content"] == "[context omitted]";
        if first && !second {
            None
        } else {
            Some(runner(request))
        }
    })
    .unwrap();
    let pair = report.pair.as_ref().unwrap();
    assert_eq!(pair.difference_pp, Some(-100.0));
    assert_eq!(pair.interaction_pp, None);
    replay::validate_report(&report, &runs).unwrap();

    runs[0].calls[0].context[1].text = runs[0].calls[0].context[0].text.clone();
    case.interventions[1].pointer = case.interventions[0].pointer.clone();
    assert!(replay::execute(&case, &runs, |_| panic!("runner must not be called")).is_err());
    case.interventions.pop();
    assert!(
        replay::execute(&case, &runs, |_| panic!("runner must not be called"))
            .unwrap_err()
            .contains("exactly two")
    );
}

#[test]
fn groups_pair_tests_across_scopes_regardless_of_the_order_of_the_two_items() {
    let (runs, mut case) = inputs();
    case.test_pair = true;
    let first = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    case.location.run_id = "other-store".into();
    case.interventions.reverse();
    let second = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    let reports = [first, second];
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    assert_eq!(groups.len(), 3);
    let pair = groups
        .iter()
        .find(|group| matches!(group.comparison.target, ComparisonTarget::Pair { .. }))
        .unwrap();
    assert_eq!(pair.occurrences.len(), 2);
    assert_eq!(pair.occurrences[0].organization_id, Some("demo-retail"));
    assert_eq!(pair.occurrences[1].organization_id, Some("second-retailer"));
    assert!(pair.occurrences.iter().all(|item| matches!(
        item.target,
        OccurrenceTarget::Pair {
            context_ids: ["return-preference", "returns-policy"],
            interaction_pp: Some(0.0),
        }
    )));
}

#[test]
fn replaces_json_from_imported_otlp_and_checks_the_recorded_object() {
    let capture = serde_json::from_str(include_str!("../fixtures/otlp.json")).unwrap();
    let mut runs = otlp::import(&capture).unwrap();
    let call = &runs[0].calls[0];
    let tool = &call.context[2];
    let original: Value = serde_json::from_str(tool.text.as_ref().unwrap()).unwrap();
    let mut changed = original.clone();
    changed["description"] = json!("Check stock by SKU");
    let decision = &call.decisions[0];
    let mut case = replay::Case {
        version: 1,
        id: "change-description".into(),
        runner_id: "Fixture".into(),
        location: replay::Location {
            run_id: runs[0].id.clone(),
            call_id: call.id.clone(),
            decision_id: decision.id.clone(),
        },
        request: json!({"tool/definitions":[original], "temperature":0}),
        samples: 1,
        interventions: vec![replay::Intervention {
            context_id: tool.id.clone(),
            pointer: "/tool~1definitions/0".into(),
            operation: Operation::ReplaceJson,
            text_range: None,
            replacement: serde_json::to_string_pretty(&changed).unwrap(),
        }],
        test_pair: false,
        negative_control_context_id: None,
        match_rule: MatchRule::Exact,
        sampling_design: replay::SamplingDesign::Unspecified,
    };
    let mut requests = Vec::new();
    let report = replay::execute(&case, &runs, |request| {
        requests.push(request.clone());
        Some(replay::Choice {
            kind: decision.kind.clone(),
            label: decision.label.clone(),
            outcome: decision.outcome.clone(),
        })
    })
    .unwrap();
    assert_eq!(
        requests,
        [
            case.request.clone(),
            json!({"tool/definitions":[changed], "temperature":0})
        ]
    );
    assert_eq!(
        report.variants[0].intervention.replacement,
        changed.to_string()
    );

    case.interventions[0].replacement = serde_json::to_string_pretty(&original).unwrap();
    assert!(
        replay::execute(&case, &runs, |_| panic!("unchanged edit should fail"))
            .unwrap_err()
            .contains("unchanged")
    );
    case.interventions[0].replacement = "[]".into();
    assert!(
        replay::execute(&case, &runs, |_| panic!("changed type should fail"))
            .unwrap_err()
            .contains("container type")
    );
    case.interventions[0].replacement = changed.to_string();
    case.request["tool/definitions"][0]["description"] = json!("Other description");
    assert!(
        replay::execute(&case, &runs, |_| panic!("mismatched input should fail"))
            .unwrap_err()
            .contains("differs from the recorded context")
    );

    runs[0].calls[0].context[2].text = Some(serde_json::to_string_pretty(&original).unwrap());
    replay::validate_report(&report, &runs).unwrap();
    runs[0].calls[0].context[2].text = Some(changed.to_string());
    assert!(replay::validate_report(&report, &runs).is_err());
}

#[test]
fn removes_array_entries_in_original_coordinates_and_rejects_overlapping_pair_edits() {
    let (mut runs, mut case) = inputs();
    let tools: Vec<_> = (0..12)
        .map(|i| json!({"name":format!("tool_{i}")}))
        .collect();
    case.request["tools"] = json!(tools);
    for (index, tool_index) in [2, 10].into_iter().enumerate() {
        let item = &mut runs[0].calls[0].context[index];
        item.text = Some(tools[tool_index].to_string());
        item.kind = "tool_definition".into();
        let intervention = &mut case.interventions[index];
        intervention.pointer = format!("/tools/{tool_index}");
        intervention.operation = Operation::RemoveJson;
        intervention.replacement.clear();
    }
    case.test_pair = true;
    case.samples = 1;
    let mut requests = Vec::new();
    let report = replay::execute(&case, &runs, |request| {
        requests.push(request.clone());
        Some(runner(request))
    })
    .unwrap();
    let without = |removed: &[usize]| {
        let mut expected = case.request.clone();
        expected["tools"] = json!(tools
            .iter()
            .enumerate()
            .filter_map(|(i, tool)| (!removed.contains(&i)).then_some(tool))
            .collect::<Vec<_>>());
        expected
    };
    assert_eq!(
        requests,
        [
            case.request.clone(),
            without(&[2]),
            without(&[10]),
            without(&[2, 10])
        ]
    );
    replay::validate_report(&report, &runs).unwrap();

    // A sibling text edit must still address its original array index when a JSON entry is removed.
    runs[0].calls[0].context[1].text = Some("tool_10".into());
    case.interventions[1].pointer = "/tools/10/name".into();
    case.interventions[1].operation = Operation::ReplaceText;
    case.interventions[1].replacement = "different-tool-name".into();
    let mut mixed = Vec::new();
    replay::execute(&case, &runs, |request| {
        mixed.push(request.clone());
        Some(runner(request))
    })
    .unwrap();
    let mut replaced = case.request.clone();
    replaced["tools"][10]["name"] = json!("different-tool-name");
    let mut both = without(&[2]);
    both["tools"][9]["name"] = json!("different-tool-name");
    assert_eq!(mixed, [case.request.clone(), without(&[2]), replaced, both]);

    runs[0].calls[0].context[1].text = Some("tool_2".into());
    case.interventions[1].pointer = "/tools/2/name".into();
    assert!(
        replay::execute(&case, &runs, |_| panic!("overlapping edits should fail"))
            .unwrap_err()
            .contains("non-overlapping")
    );
}

#[test]
fn keeps_request_member_order_in_baseline_and_after_object_removal() {
    let (mut runs, mut case) = inputs();
    case.samples = 1;
    case.request =
        serde_json::from_str(r#"{"z":3,"remove/~me":{"z":1,"a":2},"b":{"z":4,"a":5},"a":6}"#)
            .unwrap();
    runs[0].calls[0].context[0].text = Some(r#"{"a":2,"z":1}"#.into());
    case.interventions.truncate(1);
    case.interventions[0].pointer = "/remove~1~0me".into();
    case.interventions[0].operation = Operation::RemoveJson;
    case.interventions[0].replacement.clear();
    let mut requests = Vec::new();
    replay::execute(&case, &runs, |request| {
        requests.push(request.to_string());
        Some(replay::Choice {
            kind: "tool_call".into(),
            label: "check_return_eligibility".into(),
            outcome: json!({"window_days":30}),
        })
    })
    .unwrap();
    assert_eq!(
        requests,
        [
            r#"{"z":3,"remove/~me":{"z":1,"a":2},"b":{"z":4,"a":5},"a":6}"#,
            r#"{"z":3,"b":{"z":4,"a":5},"a":6}"#,
        ]
    );
}

#[test]
fn indexes_replays_only_at_the_exact_context_and_decision() {
    let (mut runs, mut case) = inputs();
    let mut another = runs[0].calls[0].decisions[0].clone();
    another.id = "review".into();
    runs[0].calls[0].decisions.push(another);
    case.test_pair = true;
    let pair = replay::execute(&case, &runs, |request| {
        let policy_removed = request["messages"][0]["content"] == "[context omitted]";
        let preference_removed = request["messages"][1]["content"] == "[context omitted]";
        if policy_removed && !preference_removed {
            None
        } else {
            Some(runner(request))
        }
    })
    .unwrap();
    case.test_pair = false;
    case.interventions.truncate(1);
    case.location.decision_id = "review".into();
    let review = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    case.location.run_id = "damaged-item".into();
    case.location.decision_id = "eligibility".into();
    let other = replay::execute(&case, &runs, |request| Some(runner(request))).unwrap();
    let reports = [pair, review, other];
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    let context = replay_effects::by_context(&groups);
    assert_eq!(context.len(), 4);
    for item in &context {
        for reference in &item.effects {
            let occurrence = &groups[reference.group_index].occurrences[reference.occurrence_index];
            assert_eq!(occurrence.location, item.location);
            match &occurrence.target {
                OccurrenceTarget::Single { context_id } => assert_eq!(*context_id, item.context_id),
                OccurrenceTarget::Pair { context_ids, .. } => {
                    assert_eq!(item.location, &reports[0].location);
                    assert!(context_ids.contains(&item.context_id));
                }
                OccurrenceTarget::Control { .. } => {
                    panic!("a control contrast is not an individual effect")
                }
            }
        }
    }
    let policy = context
        .iter()
        .find(|item| item.location == &reports[0].location && item.context_id == "returns-policy")
        .unwrap();
    assert_eq!(policy.effects.len(), 2);
    assert!(policy.effects.iter().any(|reference| {
        let group = &groups[reference.group_index];
        matches!(group.comparison.target, ComparisonTarget::Single(_))
            && group.occurrences[reference.occurrence_index]
                .difference_pp
                .is_none()
    }));
    assert!(policy.effects.iter().any(|reference| {
        let group = &groups[reference.group_index];
        matches!(group.comparison.target, ComparisonTarget::Pair { .. })
            && group.occurrences[reference.occurrence_index].difference_pp == Some(-100.0)
    }));
    assert!(context
        .iter()
        .filter(|item| item.location != &reports[0].location)
        .all(|item| item.effects.len() == 1));
}

#[test]
fn matches_selected_fields_across_output_formats_and_groups_compatible_rules() {
    let (mut runs, mut case) = inputs();
    let recorded = &mut runs[0].calls[0].decisions[0];
    recorded.outcome = json!({"window_days":30,"eligible":true,"message":"Recorded explanation"});
    let simulate = |request: &Value| {
        let mut proposed = runner(request);
        let outcome = json!({
            "window_days": proposed.outcome["window_days"],
            "eligible": true,
            "message":"New explanation",
        });
        proposed.outcome = Value::String(outcome.to_string());
        Some(proposed)
    };
    let exact = replay::execute(&case, &runs, simulate).unwrap();
    assert_eq!(exact.baseline_summary.matched, 0);
    assert_eq!(exact.variants[0].difference_pp, None);

    case.match_rule = MatchRule::OutcomeFields {
        pointers: vec!["/window_days".into(), "/eligible".into()],
    };
    case.id = "fields-a".into();
    let fields = replay::execute(&case, &runs, simulate).unwrap();
    assert_eq!(fields.baseline_summary.matched, 4);
    assert_eq!(fields.variants[0].difference_pp, Some(-100.0));
    assert_eq!(fields.variants[1].difference_pp, Some(0.0));
    assert!(
        matches!(&fields.match_rule, MatchRule::OutcomeFields { pointers } if pointers == &["/eligible", "/window_days"])
    );
    replay::validate_report(&fields, &runs).unwrap();
    case.match_rule = MatchRule::OutcomeFields {
        pointers: vec!["/eligible".into(), "/window_days".into()],
    };
    case.id = "fields-b".into();
    let mut reversed = replay::execute(&case, &runs, simulate).unwrap();
    if let MatchRule::OutcomeFields { pointers } = &mut reversed.match_rule {
        pointers.reverse();
    }
    replay::validate_report(&reversed, &runs).unwrap();

    case.match_rule = MatchRule::KindAndLabel;
    let label = replay::execute(&case, &runs, simulate).unwrap();
    assert_eq!(label.variants[0].difference_pp, Some(0.0));
    let reports = [exact, fields, reversed, label];
    let groups = replay_effects::group(&runs.iter().collect::<Vec<_>>(), &reports).unwrap();
    assert_eq!(groups.len(), 6);
    assert!(groups
        .iter()
        .all(|group| match group.comparison.match_rule {
            MatchRule::OutcomeFields { .. } => group.occurrences.len() == 2,
            _ => group.occurrences.len() == 1,
        }));
    let distributions =
        serde_json::to_value(replay::outcome_distributions(&reports).unwrap()).unwrap();
    for index in [1, 2] {
        assert_eq!(distributions[index]["replay_index"], index);
        assert_eq!(distributions[index]["buckets"].as_array().unwrap().len(), 2);
        let alternate = &distributions[index]["buckets"][1];
        assert_eq!(alternate["alternative_id"], 0);
        assert_eq!(
            alternate["selection"],
            json!({"/eligible":true,"/window_days":60})
        );
        assert_eq!(alternate["counts"], json!([{"condition":1,"count":4}]));
    }
    assert_eq!(distributions[3]["buckets"].as_array().unwrap().len(), 1);
}

#[test]
fn captures_values_from_paraphrases_and_keeps_ambiguous_trials_unscored() {
    let (mut runs, mut case) = inputs();
    runs[0].calls[0].decisions[0].outcome = json!("Return window is 30 days");
    case.match_rule = MatchRule::TextCapture {
        pattern: r"\b([0-9]+) days\b".into(),
    };
    case.test_pair = true;
    let mut failed = false;
    let mut report = replay::execute(&case, &runs, |request| {
        let mut result = runner(request);
        let policy_removed = request["messages"][0]["content"] == "[context omitted]";
        let preference_removed = request["messages"][1]["content"] == "[context omitted]";
        if policy_removed && !preference_removed && !failed {
            failed = true;
            return None;
        }
        result.outcome = json!(match (policy_removed, preference_removed) {
            (true, false) => "30 days or 60 days",
            (true, true) => "Please use 60 days for a return",
            _ => "Please use 30 days for a return",
        });
        Some(result)
    })
    .unwrap();
    assert_eq!(report.baseline_summary.matched, 4);
    assert_eq!(report.variants[0].summary.unscored, 3);
    assert_eq!(report.variants[0].summary.completed, 3);
    assert_eq!(report.variants[0].difference_pp, None);
    assert_eq!(report.variants[1].difference_pp, Some(0.0));
    assert_eq!(report.pair.as_ref().unwrap().difference_pp, Some(-100.0));
    assert_eq!(report.pair.as_ref().unwrap().interaction_pp, None);
    replay::validate_report(&report, &runs).unwrap();
    let distributions =
        serde_json::to_value(replay::outcome_distributions(std::slice::from_ref(&report)).unwrap())
            .unwrap();
    let buckets = distributions[0]["buckets"].as_array().unwrap();
    assert_eq!(buckets[0]["status"], "match");
    assert_eq!(buckets[0]["selection"], "30");
    assert_eq!(
        buckets[0]["counts"],
        json!([{"condition":0,"count":4},{"condition":2,"count":4}])
    );
    assert_eq!(buckets[1]["status"], "failed");
    assert_eq!(buckets[1]["counts"], json!([{"condition":1,"count":1}]));
    assert_eq!(buckets[2]["status"], "unscored");
    assert_eq!(buckets[2]["counts"], json!([{"condition":1,"count":3}]));
    assert_eq!(buckets[3]["status"], "alternative");
    assert_eq!(buckets[3]["selection"], "60");
    assert_eq!(buckets[3]["example"], json!({"condition":3,"trial":0}));
    report.variants[0].summary.unscored = 0;
    assert!(replay::validate_report(&report, &runs)
        .unwrap_err()
        .contains("replay scores"));
}

#[test]
fn validates_the_match_rule_against_recorded_evidence_before_sampling() {
    let (runs, mut case) = inputs();
    for rule in [
        MatchRule::OutcomeFields { pointers: vec![] },
        MatchRule::OutcomeFields {
            pointers: vec!["/window_days".into(), "/window_days".into()],
        },
        MatchRule::OutcomeFields {
            pointers: vec!["/does_not_exist".into()],
        },
        MatchRule::OutcomeFields {
            pointers: vec!["/escaped~2wrong".into()],
        },
        MatchRule::TextCapture {
            pattern: "(".into(),
        },
        MatchRule::TextCapture {
            pattern: "([0-9]+)".into(),
        },
    ] {
        case.match_rule = rule;
        assert!(replay::execute(&case, &runs, |_| panic!(
            "invalid rule should prevent sampling"
        ))
        .is_err());
    }
}

#[test]
fn rescores_saved_trials_and_preserves_their_original_source() {
    let (mut runs, mut case) = inputs();
    runs[0].calls[0].decisions[0].outcome = json!("Return window: 30 days");
    case.test_pair = true;
    let original = replay::execute(&case, &runs, |request| {
        let mut proposed = runner(request);
        let days = proposed.outcome["window_days"].as_u64().unwrap();
        proposed.outcome = json!(format!("Please use {days} days"));
        Some(proposed)
    })
    .unwrap();
    let original_json = serde_json::to_value(&original).unwrap();
    assert_eq!(original.baseline_summary.matched, 0);
    let report = replay::rescore(
        original,
        &runs,
        "quantity-only",
        MatchRule::TextCapture {
            pattern: r"\b([0-9]+) days\b".into(),
        },
    )
    .unwrap();
    assert_eq!(report.rescored_from.as_deref(), Some(case.id.as_str()));
    assert_eq!(report.baseline_summary.matched, 4);
    let pair = report.pair.as_ref().unwrap();
    assert_eq!(pair.interaction_pp, Some(0.0));
    let updated = serde_json::to_value(&report).unwrap();
    for key in [
        "location",
        "recorded",
        "baseline",
        "baseline_request_sha256",
    ] {
        assert_eq!(updated[key], original_json[key]);
    }
    assert_eq!(
        updated["variants"][0]["trials"],
        original_json["variants"][0]["trials"]
    );
    assert_eq!(updated["pair"]["trials"], original_json["pair"]["trials"]);
    let second =
        replay::rescore(report, &runs, "tool-label-only", MatchRule::KindAndLabel).unwrap();
    assert_eq!(second.rescored_from.as_deref(), Some(case.id.as_str()));
    assert_eq!(second.variants[0].difference_pp, Some(0.0));
    replay::validate_report(&second, &runs).unwrap();
}
