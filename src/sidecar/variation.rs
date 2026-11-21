//! Compare scored single-source edits across distinct captured requests.

use a2context::{replay::Intervention, Availability, Call, RecordedRequest, RequestFragment};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Input<'a> {
    pub(super) digest: String,
    order_design: Option<String>,
    pub(super) positions: BTreeMap<&'a str, (&'a RequestFragment, usize, usize)>,
}

impl<'a> Input<'a> {
    pub(super) fn new(call: &'a Call, captured: &RecordedRequest) -> Result<Self, String> {
        let mut request: Value = serde_json::from_str(&captured.body).map_err(|e| e.to_string())?;
        let mut canonical = request.clone();
        canonical.sort_all_objects();
        let digest = format!("{:x}", Sha256::digest(canonical.to_string()));

        let mut fields = BTreeMap::<_, Vec<_>>::new();
        for source in &call.context {
            if let (Some(name), Some(fragment)) = (&source.source_id, &source.request_fragment) {
                if source.availability == Availability::Included {
                    fields
                        .entry(&fragment.pointer)
                        .or_default()
                        .push((source, name, fragment));
                }
            }
        }
        let mut positions = BTreeMap::new();
        let mut layouts = Vec::new();
        for (pointer, mut sources) in fields {
            sources.sort_by_key(|(_, _, f)| f.text_range.start_byte);
            let size = sources.len();
            let Some(field) = request.pointer_mut(pointer) else {
                continue;
            };
            let Some(text) = field.as_str() else { continue };
            let mut cursor = 0;
            let mut gaps = Vec::new();
            let mut inventory = BTreeSet::new();
            for (source, name, fragment) in &sources {
                let range = &fragment.text_range;
                let Some(gap) = text.get(cursor..range.start_byte) else {
                    break;
                };
                let Some(content) = text.get(range.start_byte..range.end_byte) else {
                    break;
                };
                if source.text.as_deref() != Some(content)
                    || !inventory.insert((name, &source.source_version, content))
                {
                    break;
                }
                gaps.push(gap);
                cursor = range.end_byte;
            }
            if size < 2 || gaps.len() != size {
                continue;
            }
            gaps.push(&text[cursor..]);
            layouts.push(json!([pointer, gaps, inventory]));
            *field = Value::Null;
            for (index, (source, _, fragment)) in sources.into_iter().enumerate() {
                positions.insert(source.id.as_str(), (fragment, index + 1, size));
            }
        }
        // Slots keep their exact intervening bytes. Only the order of the
        // identified fragments within each eligible field is discarded.
        let order_design = (!layouts.is_empty()).then(|| {
            format!(
                "{:x}",
                Sha256::digest(json!([request, layouts]).to_string())
            )
        });
        Ok(Self {
            digest,
            order_design,
            positions,
        })
    }

    pub(super) fn annotate(&self, detail: &mut Value, edit: &Intervention) {
        detail["input_digest"] = json!(self.digest);
        if let Some((fragment, index, size)) = self.positions.get(edit.context_id.as_str()) {
            if edit.pointer == fragment.pointer
                && edit.text_range.as_ref() == Some(&fragment.text_range)
            {
                detail["fragment_order"] = json!({"design":self.order_design,"pointer":edit.pointer,
                    "position":index,"total":size});
            }
        }
    }
}

// Factor and comparison keep the source, edit, model, runner, and scoring rule
// together. Equal trial counts keep a one-shot comparison apart from a batch.
// Repeated observations of a body count once. An input with both results
// cannot supply an example of a consistently changed or equal comparison.
pub(super) const BY_INPUT: &str = ", input_results AS (
    SELECT factor, group_id, json_extract(detail,'$.input_digest') AS input,
        json_extract(detail,'$.measurement.baseline.attempted') AS baseline_trials,
        json_extract(detail,'$.measurement.variant.attempted') AS edit_trials,
        min(changed) AS low, max(changed) AS high,
        min(json_array(trace_id,n)) AS example
    FROM events WHERE kind='replay' AND json_extract(detail,'$.input_digest') IS NOT NULL
    GROUP BY factor,group_id,input,baseline_trials,edit_trials
), variations AS (
    SELECT factor, group_id, baseline_trials, edit_trials, sum(low=1) AS changed_inputs,
        sum(high=0) AS equal_inputs, sum(low!=high) AS mixed_inputs,
        min(example) FILTER (WHERE low=1) AS changed_example,
        min(example) FILTER (WHERE high=0) AS equal_example
    FROM input_results GROUP BY factor,group_id,baseline_trials,edit_trials
    HAVING min(high)=0 AND max(low)=1
)";

// Restrict candidates to receipts for both representative traces before checking
// their full comparison digest. CROSS JOIN fixes the lookup order so untested
// candidates never hash either observation. Each pair uses its latest receipt.
pub(super) const TESTED_POSITIONS: &str = ", measured_positions AS (
    SELECT v.factor,v.group_id,v.baseline_trials,v.edit_trials,r.pair,r.summary,r.rowid AS receipt,
        (json_extract(r.summary,'$.present.changed')=2 AND json_extract(r.summary,'$.replaced.compared')=2
         AND json_extract(r.summary,'$.replaced.changed')=0) AS isolated
    FROM variations v
    CROSS JOIN position_results r INDEXED BY position_results_first ON r.first_trace=json_extract(v.changed_example,'$[0]')
        AND r.second_trace=json_extract(v.equal_example,'$[0]')
    CROSS JOIN observations a ON a.trace_id=r.first_trace AND a.n=json_extract(v.changed_example,'$[1]')
    CROSS JOIN observations b ON b.trace_id=r.second_trace AND b.n=json_extract(v.equal_example,'$[1]')
    WHERE r.rowid=(SELECT max(newest.rowid) FROM position_results newest WHERE newest.pair=r.pair)
      AND r.pair=position_pair_id(v.factor,v.group_id,a.detail,b.detail)
), tested_factors AS (
    SELECT *,row_number() OVER (PARTITION BY factor ORDER BY isolated DESC,receipt DESC) AS priority
    FROM measured_positions
)";

// Resolve only the two representative observations for each returned comparison.
pub(super) const EXAMPLES: &str = " SELECT json_object(
    'factor',v.factor,'group',v.group_id,'changed_inputs',v.changed_inputs,
    'equal_inputs',v.equal_inputs,'mixed_inputs',v.mixed_inputs,
    'changed_example',json(a.detail),'equal_example',json(b.detail))
    FROM variations v
    JOIN observations a ON a.trace_id=json_extract(v.changed_example,'$[0]') AND a.n=json_extract(v.changed_example,'$[1]')
    JOIN observations b ON b.trace_id=json_extract(v.equal_example,'$[0]') AND b.n=json_extract(v.equal_example,'$[1]')
    ORDER BY min(v.changed_inputs,v.equal_inputs) DESC,v.changed_inputs+v.equal_inputs DESC,
        v.factor,v.group_id,v.baseline_trials,v.edit_trials LIMIT ? OFFSET ?";

pub(super) fn matched_order(row: &Value) -> bool {
    let first = &row["changed_example"]["fragment_order"];
    let second = &row["equal_example"]["fragment_order"];
    first["design"].is_string()
        && first["design"] == second["design"]
        && first["pointer"] == second["pointer"]
        && first["position"] != second["position"]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(parts: [&str; 2], separator: &str) -> Call {
        let request = json!({"text":parts.join(separator), "temperature":0});
        let mut start = 0;
        let context: Vec<_> = parts
            .iter()
            .map(|text| {
                let end = start + text.len();
                let item = json!({"id":text,"kind":"message","text":text,"source_id":text,
                "availability":"included","request_fragment":{"pointer":"/text",
                "text_range":{"start_byte":start,"end_byte":end}},"evidence":{"field":"request"}});
                start = end + separator.len();
                item
            })
            .collect();
        serde_json::from_value(json!({"id":"call","recorded_request":{"body":request.to_string(),
            "evidence":{"field":"request"}},"context":context,"decisions":[],"tool_results":[],"relationships":[]})).unwrap()
    }

    #[test]
    fn order_proof_preserves_other_input_bytes_and_source_identity() {
        let first = call(["café", "tea"], " | ");
        let reversed = call(["tea", "café"], " | ");
        let input = |call: &Call| {
            Input::new(call, call.recorded_request.as_ref().unwrap())
                .unwrap()
                .order_design
        };
        let design = input(&first);
        assert!(design.is_some());
        assert_eq!(design, input(&reversed));
        assert_ne!(design, input(&call(["tea", "café"], " / ")));
        let mut renamed = reversed.clone();
        renamed.context[0].source_version = Some("2".into());
        assert_ne!(design, input(&renamed));
        let mut settings = reversed.clone();
        let body = &mut settings.recorded_request.as_mut().unwrap().body;
        *body = body.replace("\"temperature\":0", "\"temperature\":1");
        assert_ne!(design, input(&settings));
        let mut overlap = reversed.clone();
        overlap.context[1]
            .request_fragment
            .as_mut()
            .unwrap()
            .text_range
            .start_byte = 1;
        assert!(input(&overlap).is_none());
    }
}
