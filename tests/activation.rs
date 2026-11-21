use a2context::{activation::Report, Run};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn point_report_binds_the_source_and_checks_both_controls() {
    let mut runs: Vec<Run> =
        serde_json::from_str(include_str!("../fixtures/support.json")).unwrap();
    let call = &mut runs[0].calls[0];
    call.decisions[0].outcome = json!(30);
    let source = &call.context[0];
    let digest = format!("{:x}", Sha256::digest(source.text.as_ref().unwrap()));
    let report = json!({
        "version":1, "id":"synthetic-point", "case_id":"return-window",
        "model_id":"fixture", "model_revision":"1", "runtime":"mock",
        "prompt_sha256":"0".repeat(64), "tokenizer_sha256":"1".repeat(64),
        "location":{"run_id":"return-eligibility","call_id":"check","decision_id":"eligibility"},
        "readout":"next_token_log_odds","alternatives":["30","60"],"choice_token_ids":[1,2],
        "prediction_position":3,"layer_count":2,"baseline":{"answer":30,"margin":1.0},
        "sources":[{"context_id":source.id,"captured_text_sha256":digest,
            "token_range":[0,3],"edited_token_range":[1,2],"before":"30","after":"60",
            "edited_outcome":{"answer":60,"margin":-1.0}}],
        "checks":{"layer":1,"source_context_id":source.id,"clean_patch_margin":1.0,"full_donor_margin":-1.0},
        "sweeps":[{"source_context_id":source.id,"target":"source_tokens","token_range":[1,2],"margins":[0.0,-1.0]}]
    });
    let validate = |value: Value| {
        serde_json::from_value::<Report>(value)
            .unwrap()
            .validate(&runs)
    };
    validate(report.clone()).unwrap();
    for (path, replacement, error) in [
        (
            "/sources/0/captured_text_sha256",
            json!("2".repeat(64)),
            "source text differs",
        ),
        (
            "/checks/clean_patch_margin",
            json!(-1.0),
            "instrumentation controls",
        ),
        (
            "/checks/full_donor_margin",
            json!(1.0),
            "instrumentation controls",
        ),
        ("/sweeps/0/margins", json!([0.0]), "layer count"),
    ] {
        let mut invalid = report.clone();
        *invalid.pointer_mut(path).unwrap() = replacement;
        assert!(validate(invalid).unwrap_err().contains(error));
    }
}
