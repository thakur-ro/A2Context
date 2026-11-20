//! Prepare a controlled source-position experiment from two verified replays.

use super::{store::Store, variation::Input};
use crate::Result;
use nexusweave::{
    replay::{
        position::{Background, Condition, Experiment, Plan},
        Intervention, Location,
    },
    TextRange,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text))
}

pub(super) fn pair_id(row: &Value) -> String {
    comparison_digest(
        &row["factor"],
        &row["group"],
        [&row["changed_example"], &row["equal_example"]],
    )
}

fn comparison_digest(factor: &Value, group: &Value, examples: [&Value; 2]) -> String {
    let references = examples.map(|example| {
        let m = &example["measurement"];
        json!([
            m["location"],
            m["replay_id"],
            m["context_id"],
            example["input_digest"],
            example["fragment_order"]
        ])
    });
    digest(&json!([factor, group, references]).to_string())
}

// The overview and plan export must identify the same two observations after filtering.
pub(super) fn register_pair_id(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(
        "position_pair_id",
        4,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let detail = |i| -> rusqlite::Result<Value> {
                serde_json::from_str(ctx.get_raw(i).as_str()?)
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))
            };
            Ok(comparison_digest(
                &json!(ctx.get_raw(0).as_str()?),
                &json!(ctx.get_raw(1).as_str()?),
                [&detail(2)?, &detail(3)?],
            ))
        },
    )
}

pub(super) fn prepare(store: &Store, row: &Value) -> Result<Plan> {
    let mut requests = BTreeMap::new();
    let backgrounds = [
        background(
            store,
            &row["changed_example"],
            &row["equal_example"],
            &mut requests,
        )?,
        background(
            store,
            &row["equal_example"],
            &row["changed_example"],
            &mut requests,
        )?,
    ];
    let example = &row["changed_example"];
    Ok(Plan {
        version: 1,
        experiment: Experiment::SourcePosition,
        pair: pair_id(row),
        source: example["sources"][0].clone(),
        runner_id: example["runner"]
            .as_str()
            .ok_or("missing measured runner")?
            .into(),
        match_rule: serde_json::from_value(example["match_rule"].clone())?,
        backgrounds,
        requests,
    })
}

fn background(
    store: &Store,
    detail: &Value,
    other: &Value,
    requests: &mut BTreeMap<String, String>,
) -> Result<Background> {
    let m = &detail["measurement"];
    let location: Location = serde_json::from_value(m["location"].clone())?;
    let run = store
        .trace(&location.run_id)?
        .ok_or("captured request is unavailable")?;
    let call = run
        .calls
        .iter()
        .find(|c| c.id == location.call_id)
        .ok_or("captured call is unavailable")?;
    let captured = call
        .recorded_request
        .as_ref()
        .ok_or("request body is unavailable")?;
    let reports = store.replays(&run.id)?;
    let report = reports
        .iter()
        .find(|r| r.location == location && r.id == m["replay_id"])
        .ok_or("measured replay is unavailable")?;
    let edit = &report
        .variants
        .iter()
        .find(|v| v.intervention.context_id == m["context_id"])
        .ok_or("tested source edit is unavailable")?
        .intervention;
    let input = Input::new(call, captured)?;
    let mut checked = json!({});
    input.annotate(&mut checked, edit);
    if checked["fragment_order"].is_null()
        || checked["fragment_order"] != detail["fragment_order"]
        || checked["input_digest"] != detail["input_digest"]
    {
        return Err("fragment layout differs from the measured comparison".into());
    }
    let original: Value = serde_json::from_str(&captured.body)?;
    if digest(&original.to_string()) != report.baseline_request_sha256 {
        return Err("captured body differs from the measured replay".into());
    }
    let mut slots: Vec<_> = input
        .positions
        .values()
        .filter(|(f, _, _)| f.pointer == edit.pointer)
        .map(|(f, _, _)| f.text_range)
        .collect();
    slots.sort_by_key(|r| r.start_byte);
    let from = input.positions[edit.context_id.as_str()].1;
    let to = other["fragment_order"]["position"]
        .as_u64()
        .ok_or("missing destination slot")? as usize;
    Ok(Background {
        location,
        replay_id: report.id.clone(),
        context_id: edit.context_id.clone(),
        pointer: edit.pointer.clone(),
        original_position: from,
        captured_request_sha256: report.baseline_request_sha256.clone(),
        conditions: conditions(&original, edit, &slots, from, to, requests)?,
    })
}

fn conditions(
    original: &Value,
    edit: &Intervention,
    slots: &[TextRange],
    from: usize,
    to: usize,
    requests: &mut BTreeMap<String, String>,
) -> Result<[Condition; 4]> {
    let text = original
        .pointer(&edit.pointer)
        .and_then(Value::as_str)
        .ok_or("expected a text field")?;
    if from == 0 || to == 0 || from > slots.len() || to > slots.len() || from == to {
        return Err("expected two distinct positions in the captured field".into());
    }
    let parts: Vec<&str> = slots
        .iter()
        .map(|r| &text[r.start_byte..r.end_byte])
        .collect();
    let mut moved = parts.clone();
    let selected = moved.remove(from - 1);
    moved.insert(to - 1, selected);
    Ok(
        [(from, false), (from, true), (to, false), (to, true)].map(|(position, replaced)| {
            let parts = if position == from { &parts } else { &moved };
            let mut value = String::new();
            let mut cursor = 0;
            for (i, range) in slots.iter().enumerate() {
                value.push_str(&text[cursor..range.start_byte]);
                value.push_str(if replaced && i + 1 == position {
                    &edit.replacement
                } else {
                    parts[i]
                });
                cursor = range.end_byte;
            }
            value.push_str(&text[cursor..]);
            let mut request = original.clone();
            *request
                .pointer_mut(&edit.pointer)
                .expect("validated text pointer") = Value::String(value);
            let body = request.to_string();
            let hash = digest(&body);
            requests.entry(hash.clone()).or_insert(body);
            Condition {
                position,
                source_replaced: replaced,
                request_sha256: hash,
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_preserves_slot_separators_settings_and_other_sources() {
        let original = json!({"messages":[{"content":"1:é |2:blue ::3:茶!"}],"tools":[]});
        let ranges = [(2, 4), (8, 12), (17, 20)].map(|(start_byte, end_byte)| TextRange {
            start_byte,
            end_byte,
        });
        let edit: Intervention = serde_json::from_value(
            json!({"context_id":"tea","operation":"replace_text_fragment",
            "pointer":"/messages/0/content","text_range":ranges[2],"replacement":"∅"}),
        )
        .unwrap();
        let mut requests = BTreeMap::new();
        let arms = conditions(&original, &edit, &ranges, 3, 1, &mut requests).unwrap();
        for (arm, expected) in arms.iter().zip([
            "1:é |2:blue ::3:茶!",
            "1:é |2:blue ::3:∅!",
            "1:茶 |2:é ::3:blue!",
            "1:∅ |2:é ::3:blue!",
        ]) {
            let body = json!({"messages":[{"content":expected}],"tools":[]}).to_string();
            assert_eq!(arm.request_sha256, digest(&body));
            assert_eq!(requests[arm.request_sha256.as_str()], body);
        }
        assert!(conditions(&original, &edit, &ranges, 3, 4, &mut requests).is_err());
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Submission {
    selection: BTreeMap<String, Value>,
    pub result: nexusweave::replay::position::Results,
}

impl Submission {
    pub fn filters(&self) -> Result<BTreeMap<String, String>> {
        self.selection
            .iter()
            .map(|(key, value)| {
                if let Some(text) = value.as_str() {
                    return Ok((key.clone(), text.to_owned()));
                }
                if matches!(key.as_str(), "baseline_trials" | "edit_trials") {
                    if let Some(count) = value.as_u64() {
                        return Ok((key.clone(), count.to_string()));
                    }
                }
                Err(format!("invalid selection value for {key}").into())
            })
            .collect()
    }
}
