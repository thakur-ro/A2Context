use nexusweave::{
    context_screening::{self as screening, patterns, Assessment, Report, Suggestion, Unscored},
    Run,
};
use serde_json::{json, Value};

const COHORT: &str = include_str!("../examples/context-screening/cohort.json");

fn cohort() -> Vec<Run> {
    serde_json::from_str(COHORT).unwrap()
}

fn query(spec: Value, runs: &[Run], excerpts: bool) -> Value {
    patterns::query(&serde_json::from_value(spec).unwrap(), runs, excerpts).unwrap()
}

fn report(run: &mut Run) -> &mut Report {
    run.calls[0]
        .context_screening
        .as_mut()
        .unwrap()
        .report
        .as_mut()
        .unwrap()
}

fn system_developer(page: &Value) -> &Value {
    page["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| {
            group["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["role"] == "system")
        })
        .unwrap()
}

#[test]
fn compares_reversed_source_pairs_in_distinct_scopes_and_opens_the_exact_capture() {
    let runs = cohort();
    let mut spec = json!({
        "version":1, "relation":"possible_conflict", "min_conversations":2,
        "selection":{"scope":{"kind":"user","organization_id":"retail-north","user_id":"routing-sample"}},
        "compare":{"scope":{"kind":"organization","organization_id":"retail-south"}}
    });
    let groups = query(spec.clone(), &runs, true);
    assert_eq!(groups["total_groups"], 2);
    let pair = system_developer(&groups);
    assert_eq!(pair["primary"]["flagged_conversations"], 2);
    assert_eq!(pair["primary"]["assessed_calls"], 2);
    assert_eq!(pair["comparison"]["flagged_conversations"], 1);
    assert_eq!(pair["comparison"]["assessed_calls"], 1);
    assert!(pair["excerpts"]
        .to_string()
        .contains("Customers cannot view"));
    assert!(!query(spec.clone(), &runs, false)
        .to_string()
        .contains("Customers"));

    spec["group"] = pair["key"].clone();
    spec["limit"] = json!(1);
    let page = query(spec.clone(), &runs, false);
    assert_eq!(page["primary"]["next_offset"], 1);
    assert_eq!(
        page["primary"]["rows"][0]["conversation_id"],
        "receipt-review-1"
    );
    let opposite = &page["comparison"]["rows"][0];
    assert_eq!(opposite["conversation_id"], "receipt-review-3");
    assert_eq!(opposite["sources"][0]["role"], "developer");
    assert_eq!(opposite["sources"][1]["role"], "system");
    assert_eq!(opposite["suggestion"], "possible_conflict");
    assert!(opposite["inspector_path"]
        .as_str()
        .unwrap()
        .contains("&screen_pair=%5B%22input%3A0%3A0%22"));
    spec["offset"] = json!(1);
    assert_eq!(
        query(spec, &runs, false)["primary"]["rows"][0]["conversation_id"],
        "receipt-review-2"
    );
}

#[test]
fn preserves_scored_unflagged_and_unscored_calls_in_comparable_coverage() {
    let mut runs = cohort();
    let later = report(&mut runs[1]);
    let pair = later
        .pairs
        .iter_mut()
        .find(|pair| pair.sources == [0, 3])
        .unwrap();
    let neutral = screening::Scores {
        contradiction: 0.0,
        entailment: 0.0,
        neutral: 1.0,
    };
    pair.assessment = Assessment::Scored {
        left_to_right: neutral,
        right_to_left: neutral,
    };
    pair.suggestion = None;
    let reversed = report(&mut runs[2]);
    let pair = reversed
        .pairs
        .iter_mut()
        .find(|pair| pair.sources == [0, 1])
        .unwrap();
    pair.assessment = Assessment::Unscored {
        reason: Unscored::TooLong,
    };
    pair.suggestion = None;

    let groups = query(
        json!({"version":1,"relation":"possible_conflict"}),
        &runs,
        false,
    );
    let pair = system_developer(&groups);
    assert_eq!(pair["primary"]["screened_calls"], 3);
    assert_eq!(pair["primary"]["assessed_calls"], 2);
    assert_eq!(pair["primary"]["flagged_calls"], 1);
    assert_eq!(pair["primary"]["unscored_calls"], 1);
    let evidence = query(
        json!({"version":1,"relation":"possible_conflict","group":pair["key"]}),
        &runs,
        false,
    );
    assert_eq!(
        evidence["primary"]["rows"][1]["assessment"]["status"],
        "scored"
    );
    assert!(evidence["primary"]["rows"][1]["suggestion"].is_null());
    assert_eq!(
        evidence["primary"]["rows"][2]["assessment"]["reason"],
        "too_long"
    );
}

#[test]
fn keeps_other_scorers_and_incomplete_capture_out_of_recurrence_counts() {
    let mut runs = cohort();
    report(&mut runs[2]).scorer.as_mut().unwrap().revision = "alternate".into();
    runs[3].calls[0].context[0].text.as_mut().unwrap().push(' ');
    let groups = query(
        json!({"version":1,"relation":"possible_conflict","min_conversations":2}),
        &runs,
        false,
    );
    let pair = system_developer(&groups);
    assert_eq!(pair["primary"]["flagged_calls"], 2);
    assert!(groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["scorer"]["revision"] != "alternate"));

    let windows = query(
        json!({
            "version":1, "relation":"possible_overlap",
            "selection":{"before":runs[1].calls[0].started_at},
            "compare":{"after":runs[1].calls[0].started_at, "scope":{"kind":"organization","organization_id":"retail-north"}}
        }),
        &runs,
        false,
    );
    assert_eq!(windows["groups"][0]["primary"]["flagged_conversations"], 1);
    assert_eq!(
        windows["groups"][0]["comparison"]["flagged_conversations"],
        1
    );
}

#[test]
fn counts_repeated_slots_within_a_call_once() {
    let mut run = cohort().remove(0);
    let call = &mut run.calls[0];
    call.context.truncate(2);
    let mut duplicate = call.context[0].clone();
    duplicate.id = "duplicate-system".into();
    call.context.push(duplicate);
    let scores = screening::Scores {
        contradiction: 1.0,
        entailment: 0.0,
        neutral: 0.0,
    };
    let result = screening::screen_call(call, &screening::Selection::default(), |batch| {
        Ok(Some(screening::ScoringResult {
            version: 1,
            scorer: screening::Scorer {
                model_id: "test".into(),
                revision: "test".into(),
                runtime: "test".into(),
            },
            pairs: batch
                .pairs
                .iter()
                .map(|pair| screening::ScoredPair {
                    id: pair.id,
                    assessment: Assessment::Scored {
                        left_to_right: scores,
                        right_to_left: scores,
                    },
                })
                .collect(),
        }))
    })
    .unwrap();
    assert_eq!(
        result
            .pairs
            .iter()
            .filter(|pair| pair.suggestion == Some(Suggestion::PossibleConflict))
            .count(),
        2
    );
    call.context_screening.as_mut().unwrap().report = Some(result);
    let page = query(
        json!({"version":1,"relation":"possible_conflict"}),
        &[run],
        false,
    );
    assert_eq!(page["total_groups"], 1);
    assert_eq!(page["groups"][0]["primary"]["pairs"], 2);
    assert_eq!(page["groups"][0]["primary"]["flagged_calls"], 1);
    assert_eq!(page["groups"][0]["primary"]["flagged_conversations"], 1);
}
