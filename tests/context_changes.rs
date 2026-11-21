use a2context::{
    context_changes::{self, Difference, Identity, OutcomeStatus, Status, TransitionResult},
    decision_changes::DecisionChangeStatus,
    inspect,
    replay::CaptureAvailability,
    Call, ContextItem, DocumentIdentity, RequestFragment, Retrieval, Run, TextRange, ToolResult,
};
use serde_json::json;

fn support() -> Vec<Run> {
    serde_json::from_str(include_str!("../fixtures/support.json")).unwrap()
}

#[test]
fn orders_a_single_trace_by_time_when_all_call_timestamps_are_known() {
    let mut runs = [support().remove(0)];
    let run = &mut runs[0];
    run.calls[0].started_at = Some("2026-05-01T09:00:02Z".into());
    run.calls[1].started_at = Some("2026-05-01T09:00:01Z".into());
    let report = inspect(&runs).unwrap().context_history;
    assert_eq!(report.context_timelines[0].calls[0].call_id, "follow-up");
    assert!(report.context_timelines[0].has_timestamps);
    runs[0].calls[1].started_at = None;
    let report = inspect(&runs).unwrap().context_history;
    assert_eq!(report.context_timelines[0].calls[0].call_id, "check");
    assert!(!report.context_timelines[0].has_timestamps);
}

#[test]
fn aligns_named_revisions_and_preserves_redactions_and_duplicate_captures() {
    let runs = support();
    let run = runs.iter().find(|run| run.id == "holiday-order").unwrap();
    let mut before = run.calls[1].clone();
    let mut after = run.calls[2].clone();
    after.context[0].source_version = Some("v13".into());
    after.context[0].text = Some("The standard window is now 45 days.".into());
    before.context[0].rule.as_mut().unwrap().value = Some(json!(9223372036854775807u64));
    after.context[0].rule.as_mut().unwrap().value = Some(json!(9223372036854775806u64));
    let mut repeated = before.context[0].clone();
    repeated.source_id = None;
    repeated.source_version = None;
    repeated.id = "repeat-1".into();
    before.context.push(repeated.clone());
    after.context.push(repeated.clone());
    repeated.id = "repeat-2".into();
    before.context.push(repeated);

    let rows = context_changes::align((run, &before), (run, &after));
    assert_eq!(rows[0].status, Status::Changed);
    assert_eq!(
        rows[0].differences,
        [Difference::Version, Difference::Rule, Difference::Text]
    );
    assert_eq!(rows[1].status, Status::CaptureChanged);
    assert!(rows[1].differences.contains(&Difference::CaptureStatus));
    assert!(!rows[1].differences.contains(&Difference::Text));
    assert_eq!(rows[2].status, Status::Ambiguous);
    assert_eq!(
        rows[2].context_ids,
        [vec!["repeat-1", "repeat-2"], vec!["repeat-1"]]
    );
}

#[test]
fn tracks_declared_request_locations_without_treating_missing_declarations_as_movement() {
    let runs = support();
    let run = &runs[0];
    let mut before = run.calls[0].clone();
    let mut after = run.calls[1].clone();
    let fragment = |start_byte| RequestFragment {
        pointer: "/messages/0/content".into(),
        text_range: TextRange {
            start_byte,
            end_byte: start_byte + 42,
        },
    };
    before.context[0].request_fragment = Some(fragment(18));
    let without = context_changes::align((run, &before), (run, &after));
    assert_eq!(without[0].status, Status::Matching);
    after.context[0].request_fragment = Some(fragment(60));
    let with = context_changes::align((run, &before), (run, &after));
    assert_eq!(with[0].status, Status::Changed);
    assert_eq!(with[0].differences, [Difference::RequestLocation]);
}

#[test]
fn limits_document_and_invocation_identity_to_the_same_trace() {
    let runs = support();
    let run = &runs[0];
    let snapshot = |call: &Call, text: &str| {
        let mut call = call.clone();
        call.trace_id = Some("one-trace".into());
        let item = |id: &str, kind: &str| -> ContextItem {
            let mut item = call.context[0].clone();
            item.id = id.into();
            item.kind = kind.into();
            item.source_id = None;
            item.text = Some(text.into());
            item
        };
        let mut document = item("document", "input_document");
        document.document_identity = Some(DocumentIdentity {
            id: "article".into(),
            evidence: document.evidence.clone(),
        });
        let mut tool = item("definition", "tool_definition");
        tool.text = Some(json!({"function":{"name":"lookup","description":text}}).to_string());
        let result = item("result", "tool_result");
        let anchor = result.evidence.clone();
        call.context = vec![document, result, tool];
        call.tool_results = vec![ToolResult {
            tool_call_id: "invocation".into(),
            outcome: None,
            evidence: anchor,
        }];
        call
    };
    let before = snapshot(&run.calls[0], "old");
    let mut after = snapshot(&run.calls[1], "new");
    let within = context_changes::align((run, &before), (run, &after));
    assert_eq!(within.len(), 3);
    assert!(within.iter().all(|row| row.status == Status::Changed));
    assert!(matches!(
        within[0].identity,
        Some(Identity::Document { id: "article" })
    ));
    assert!(matches!(
        within[1].identity,
        Some(Identity::ToolResult {
            invocation_id: "invocation"
        })
    ));
    let across = context_changes::align((run, &before), (&runs[1], &after));
    assert_eq!(
        across
            .iter()
            .filter(|row| row.status == Status::Changed)
            .count(),
        1
    );
    assert!(across
        .iter()
        .any(|row| matches!(row.identity, Some(Identity::Tool { .. }))
            && row.status == Status::Changed));
    after.tool_results.push(ToolResult {
        tool_call_id: "different".into(),
        outcome: None,
        evidence: after.tool_results[0].evidence.clone(),
    });
    assert!(context_changes::align((run, &before), (run, &after))
        .iter()
        .all(
            |row| !matches!(row.identity, Some(Identity::ToolResult { .. }))
                || row.status == Status::First
        ));
}

#[test]
fn exports_scoped_patterns_with_references_to_the_same_call_pairs() {
    let mut runs = support();
    let mut other = runs
        .iter()
        .find(|run| run.id == "holiday-order")
        .unwrap()
        .clone();
    other.id = "second-holiday".into();
    other.organization_id = Some("second-retailer".into());
    let mut steady = other.clone();
    steady.id = "steady-holiday".into();
    steady.conversation_id = Some("steady-holiday".into());
    steady.organization_id = Some("demo-retail".into());
    steady.calls = vec![steady.calls[1].clone(), steady.calls[1].clone()];
    steady.calls[1].id = "same-capture".into();
    let mut gap = steady.clone();
    gap.id = "gap-holiday".into();
    gap.conversation_id = Some("gap-holiday".into());
    gap.calls[1].context.clear();
    gap.calls[1].relationships.clear();
    runs.push(other);
    runs.push(steady);
    runs.push(gap);
    let inspection = inspect(&runs).unwrap();
    let history = inspection.context_history;
    let pattern = history
        .context_change_patterns
        .iter()
        .find(|pattern| {
            pattern.key.identity
                == Identity::Source {
                    id: "return-preference",
                }
                && pattern.key.status == Status::CaptureChanged
        })
        .unwrap();
    assert_eq!(pattern.key.versions, [vec!["v4"], vec!["v5"]]);
    assert_eq!(pattern.occurrences.len(), 2);
    let source = &history.context_source_observations[pattern.source_observation_index];
    let earlier = &source.earlier_states[pattern.earlier_state_index.unwrap()];
    assert_eq!(
        earlier.entries,
        [context_changes::EarlierEntry {
            version: Some("v4"),
            capture: CaptureAvailability::Available,
        }]
    );
    assert_eq!(earlier.pairs.len(), 3);
    let same = earlier
        .pairs
        .iter()
        .find(|pair| pair.status == Status::Matching)
        .unwrap();
    let transition = &history.context_transitions[same.transition_index];
    assert_eq!(transition.after.run_id, "steady-holiday");
    assert_eq!(
        same.context_ids,
        [vec!["return-preference"], vec!["return-preference"]]
    );
    assert!(!earlier
        .pairs
        .iter()
        .any(|pair| history.context_transitions[pair.transition_index]
            .after
            .run_id
            == "gap-holiday"));
    assert!(source.earlier_states.iter().any(|state| state
        .entries
        .first()
        .is_some_and(|entry| entry.version == Some("v3"))));
    let introduction = history
        .context_change_patterns
        .iter()
        .find(|pattern| {
            pattern.key.identity
                == Identity::Source {
                    id: "return-preference",
                }
                && pattern.key.status == Status::Second
        })
        .unwrap();
    assert_eq!(
        introduction.source_observation_index,
        pattern.source_observation_index
    );
    assert_eq!(introduction.earlier_state_index, None);
    assert_eq!(
        source
            .earlier_states
            .iter()
            .flat_map(|state| &state.pairs)
            .filter(|pair| pair.status == Status::Second)
            .count(),
        introduction.occurrences.len()
    );
    for occurrence in &pattern.occurrences {
        let pair = &history.context_transitions[occurrence.transition_index];
        assert_eq!(pair.before.run_id, pair.after.run_id);
        assert_eq!(pair.before.call_id, "duplicate");
        assert_eq!(pair.after.call_id, "redacted");
        assert_eq!(
            pair.decision_change.as_ref().unwrap().status,
            DecisionChangeStatus::Same
        );
        assert_eq!(pair.scope.organization_id, occurrence.scope.organization_id);
        let TransitionResult::Compared { changes } = &pair.result else {
            panic!("expected compared inputs")
        };
        let change = &changes[occurrence.change_index];
        assert_eq!(change.status, Status::CaptureChanged);
        assert_eq!(
            change.context_ids,
            [vec!["return-preference"], vec!["return-preference"]]
        );
    }
    let encoded = serde_json::to_value(&history).unwrap();
    assert!(encoded["context_transitions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pair| pair["state"] == "compared" && pair.get("request").is_none()));
}

#[test]
fn keeps_capture_gaps_and_uncertain_order_at_call_boundaries() {
    let runs = support();
    let mut before = runs[0].clone();
    before.conversation_id = Some("same-thread".into());
    let captured = &before.calls[0];
    let step = |id: &str, time: &str| {
        let mut call = captured.clone();
        call.id = id.into();
        call.started_at = Some(time.into());
        call.relationships.clear();
        call
    };
    let first = step("first", "2025-01-01T00:00:00Z");
    let mut retrieve = step("retrieval", "2025-01-01T00:00:01Z");
    retrieve.retrieval = Some(Retrieval {
        label: "Search".into(),
        documents: Vec::new(),
    });
    let mut tool = step("tool", "2025-01-01T00:00:02Z");
    tool.decisions[0].kind = "tool_execution".into();
    let mut missing = step("missing", "2025-01-01T00:00:03Z");
    missing.context.clear();
    let latest = step("latest", "2025-01-01T00:00:04Z");
    let mut after = before.clone();
    after.id = "zz-other-trace".into();
    after.calls = vec![
        step("tied", "2025-01-01T00:00:04Z"),
        step("later", "2025-01-01T00:00:05Z"),
    ];
    before.calls = vec![first, retrieve, tool, missing, latest];
    let mut traces = [before, after];
    let view = inspect(&traces).unwrap();
    let pairs = &view.context_history.context_transitions;
    assert_eq!(pairs.len(), 4);
    let missing_pairs: Vec<_> = pairs
        .iter()
        .filter(|pair| matches!(pair.result, TransitionResult::MissingContext))
        .map(|pair| (pair.before.call_id, pair.after.call_id))
        .collect();
    assert_eq!(missing_pairs, [("first", "missing"), ("missing", "latest")]);
    assert!(pairs
        .iter()
        .filter(|pair| matches!(pair.result, TransitionResult::MissingContext))
        .all(|pair| pair.decision_change.is_some()));
    assert_eq!(
        pairs
            .iter()
            .filter(|pair| matches!(pair.result, TransitionResult::AmbiguousOrder))
            .count(),
        1
    );
    assert!(pairs
        .iter()
        .filter(|pair| matches!(pair.result, TransitionResult::AmbiguousOrder))
        .all(|pair| pair.decision_change.is_none()));
    traces[1].calls[0].started_at = Some("2025-01-01T00:00:04.000000001Z".into());
    let view = inspect(&traces).unwrap();
    let pair = view
        .context_history
        .context_transitions
        .iter()
        .find(|pair| pair.after.call_id == "tied")
        .unwrap();
    assert_eq!(pair.before.call_id, "latest");
    assert!(matches!(pair.result, TransitionResult::Compared { .. }));
}

#[test]
fn compares_structured_decision_values_without_rounding_wide_integers() {
    let runs = support();
    let mut left = runs[0].calls[0].decisions[0].clone();
    let mut right = left.clone();
    left.outcome = json!({"v": 9223372036854775807u64, "available": true});
    right.outcome = json!({"available": true, "v": 9223372036854775807u64});
    assert_eq!(
        context_changes::compare_outcomes(&left, &right),
        OutcomeStatus::Same
    );
    right.outcome["v"] = json!(9223372036854775806u64);
    assert_eq!(
        context_changes::compare_outcomes(&left, &right),
        OutcomeStatus::Different
    );
}
