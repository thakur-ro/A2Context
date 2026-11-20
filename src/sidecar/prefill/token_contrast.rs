//! Bind an optional greedy token readout to the three requests and replayed texts.
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    id: u32,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    request_sha256: String,
    answer_sha256: String,
    output_ids: Vec<u32>,
    chosen: u32,
    probabilities: [f64; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Readout {
    version: u32,
    model_id: String,
    model_revision: String,
    runtime: String,
    tokenizer_sha256: String,
    prefix: String,
    prefix_ids: Vec<u32>,
    stop_ids: Vec<u32>,
    tokens: [Token; 2],
    conditions: [Condition; 3],
}

pub(super) fn checked(
    raw: &Value,
    measured: &Value,
    review: &Value,
    checks: &[Value],
) -> Result<Value, &'static str> {
    let readout: Readout =
        serde_json::from_value(raw.clone()).map_err(|_| "Unreadable version 1 token contrast.")?;
    if readout.version != 1
        || measured["model_id"] != readout.model_id
        || measured["model_revision"] != readout.model_revision
        || measured["runtime"] != readout.runtime
        || measured["tokenizer_sha256"] != readout.tokenizer_sha256
    {
        return Err("The token contrast uses a different model, runtime or tokenizer.");
    }
    let [a, b] = &readout.tokens;
    if a.id == b.id
        || readout.stop_ids.is_empty()
        || readout.stop_ids.len() > 32
        || readout.prefix_ids.len() > 4096
        || readout.prefix.len() > 32768
        || (readout.prefix_ids.is_empty() && !readout.prefix.is_empty())
        || readout.tokens.iter().any(|t| t.text.len() > 1024)
    {
        return Err("The token contrast has invalid token or prefix metadata.");
    }
    let index = |key: &str| {
        review["witness"][key]
            .as_u64()
            .and_then(|i| checks.get(i as usize))
            .ok_or("Waiting for a matching pair of completed source edits.")
    };
    let top = index("top_check")?;
    let lower = index("lower_check")?;
    let ranked_edit = |check: &Value| {
        measured["ranked"]
            .as_array()?
            .iter()
            .find(|row| row["context_id"] == check["context_id"])
    };
    let top_edit = ranked_edit(top).ok_or("Missing higher-ranked source edit.")?;
    let lower_edit = ranked_edit(lower).ok_or("Missing lower-ranked source edit.")?;
    let requests = [
        measured["baseline_request_sha256"].as_str(),
        top_edit["request_sha256"].as_str(),
        lower_edit["request_sha256"].as_str(),
    ];
    let openings = [
        &measured["top_token_id"],
        &top_edit["top_token_id"],
        &lower_edit["top_token_id"],
    ];
    let answers = [
        &top["answer_digests"][0],
        &top["answer_digests"][1],
        &lower["answer_digests"][1],
    ];
    if top["answer_digests"][0] != lower["answer_digests"][0] {
        return Err("The two replay checks have different original answers.");
    }
    for (i, c) in readout.conditions.iter().enumerate() {
        if Some(c.request_sha256.as_str()) != requests[i]
            || answers[i].as_str() != Some(&c.answer_sha256)
        {
            return Err(
                "The token contrast does not reproduce these exact requests and text outputs.",
            );
        }
        let first = c.output_ids.first().copied().unwrap_or(c.chosen);
        if openings[i]
            .as_u64()
            .is_some_and(|id| id != u64::from(first))
        {
            return Err("The greedy rerun disagrees with the recorded opening token.");
        }
        let next = c.output_ids.get(readout.prefix_ids.len());
        if c.output_ids.len() > 4096
            || !c.output_ids.starts_with(&readout.prefix_ids)
            || c.output_ids.iter().any(|t| readout.stop_ids.contains(t))
            || !next.map_or_else(|| readout.stop_ids.contains(&c.chosen), |t| *t == c.chosen)
            || c.chosen != if i == 2 { b.id } else { a.id }
        {
            return Err("The reported greedy tokens do not first diverge at this position.");
        }
        let [p, q] = c.probabilities;
        if !(0.0..=1.0).contains(&p)
            || !(0.0..=1.0).contains(&q)
            || p + q > 1.0 + 1e-9
            || (c.chosen == a.id && (p == 0.0 || p < q))
            || (c.chosen == b.id && (q == 0.0 || q < p))
        {
            return Err("The token probabilities disagree with the reported choices.");
        }
    }
    Ok(
        json!({"position":readout.prefix_ids.len()+1,"prefix":readout.prefix,"tokens":[a.text,b.text],
        "stop":[readout.stop_ids.contains(&a.id),readout.stop_ids.contains(&b.id)],
        "probabilities":readout.conditions.map(|c| c.probabilities)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_the_exact_three_requests_outputs_prefix_and_greedy_choices() {
        let measured = json!({"model_id":"q","model_revision":"r","runtime":"cpu","tokenizer_sha256":"t",
            "baseline_request_sha256":"input","ranked":[{"context_id":"a","request_sha256":"top"},{"context_id":"b","request_sha256":"lower"}]});
        let checks = vec![
            json!({"context_id":"a","answer_digests":["17th","17th"]}),
            json!({"context_id":"b","answer_digests":["17th","17"]}),
        ];
        let review = json!({"witness":{"top_check":0,"lower_check":1}});
        let condition = |req, answer, ids: Vec<u32>, chosen, probabilities: [f64; 2]| {
            json!({
            "request_sha256":req,"answer_sha256":answer,"output_ids":ids,"chosen":chosen,"probabilities":probabilities})
        };
        let valid = json!({"version":1,"model_id":"q","model_revision":"r","runtime":"cpu","tokenizer_sha256":"t",
            "prefix":"17","prefix_ids":[1,7],"stop_ids":[9],"tokens":[{"id":3,"text":"th"},{"id":9,"text":"<end>"}],
            "conditions":[condition("input","17th",vec![1,7,3],3,[0.65,0.25]),condition("top","17th",vec![1,7,3],3,[0.81,0.1]),
                          condition("lower","17",vec![1,7],9,[0.1,0.8])]});
        let check = |v: &Value| checked(v, &measured, &review, &checks);
        assert_eq!(check(&valid).unwrap()["position"], 3);
        let mut mismatch = measured.clone();
        mismatch["ranked"][1]["top_token_id"] = json!(7);
        assert_eq!(
            checked(&valid, &mismatch, &review, &checks).unwrap_err(),
            "The greedy rerun disagrees with the recorded opening token."
        );
        for (pointer, value) in [
            ("/runtime", json!("other")),
            ("/prefix_ids/1", json!(8)),
            ("/tokens/0/id", json!(9)),
            ("/conditions/1/request_sha256", json!("different")),
            ("/conditions/2/answer_sha256", json!("wrong")),
            ("/stop_ids/0", json!(0)),
            ("/conditions/2/output_ids", json!([1, 7, 9])),
            ("/conditions/0/probabilities", json!([0.7, 0.4])),
            ("/conditions/2/probabilities", json!([0.8, 0.1])),
        ] {
            let mut corrupt = valid.clone();
            *corrupt.pointer_mut(pointer).unwrap() = value;
            assert!(check(&corrupt).is_err(), "{pointer}");
        }
    }
}
