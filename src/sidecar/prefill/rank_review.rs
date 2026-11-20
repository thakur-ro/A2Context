//! Check how often testing only the largest first-token shift would miss a proposal change.
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn review(ranked: &[Value], checks: &[Value]) -> Value {
    // The caller supplies at least two verified edits, sorted by descending divergence.
    if ranked[0]["js_bits"] == ranked[1]["js_bits"] {
        return json!({"status":"tied"});
    }
    let slots: BTreeMap<_, _> = ranked
        .iter()
        .enumerate()
        .map(|(i, row)| (row["context_id"].as_str(), i))
        .collect();
    let mut reports = BTreeMap::new();
    for (index, check) in checks.iter().enumerate() {
        let key = (
            check["runner"].as_str(),
            check["report"].as_str(),
            check["decision"].as_str(),
        );
        if let Some(&slot) = slots.get(&check["context_id"].as_str()) {
            reports
                .entry(key)
                .or_insert_with(|| vec![None; ranked.len()])[slot] = Some(index);
        }
    }

    let mut top_changed = false;
    let mut top_held = false;
    let mut all_held = false;
    let mut witness = None;
    for slots in reports.values() {
        let Some(top) = slots[0] else { continue };
        if checks[top]["changed"] == true {
            top_changed = true;
            continue;
        }
        top_held = true;
        if let Some(other) = slots[1..]
            .iter()
            .flatten()
            .find(|&&i| checks[i]["changed"] == true)
        {
            witness.get_or_insert((top, *other));
        } else if slots.iter().all(Option::is_some) {
            all_held = true;
        }
    }
    let status = if (top_changed && top_held) || (all_held && witness.is_some()) {
        "mixed"
    } else if top_changed {
        "top_changed"
    } else if witness.is_some() {
        "lower_changed"
    } else if all_held {
        "all_held"
    } else {
        "incomplete"
    };
    let mut result = json!({"status":status});
    if let Some((top, other)) = witness {
        result["witness"] = json!({"top_check":top,"lower_check":other});
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_edits_only_within_a_report_and_keeps_incomplete_repeats() {
        let ranked = vec![
            json!({"context_id":"t","js_bits":0.8}),
            json!({"context_id":"a","js_bits":0.5}),
            json!({"context_id":"b","js_bits":0.0}),
        ];
        let check = |report, id, changed| json!({"runner":"q","report":report,"decision":"d","context_id":id,"changed":changed});
        let top = check("1", "t", false);
        let lower = check("1", "b", true);
        for field in ["report", "runner", "decision"] {
            let mut different = lower.clone();
            different[field] = "different".into();
            assert_eq!(
                review(&ranked, &[top.clone(), different])["status"],
                "incomplete"
            );
        }
        let checks = [lower.clone(), check("1", "a", false), top.clone()];
        assert_eq!(
            review(&ranked, &checks),
            json!({"status":"lower_changed","witness":{"top_check":2,"lower_check":0}})
        );
        let mut repeats = checks.to_vec();
        repeats.push(check("2", "t", false));
        assert_eq!(review(&ranked, &repeats)["status"], "lower_changed");
        repeats.extend([check("2", "a", false), check("2", "b", false)]);
        assert_eq!(review(&ranked, &repeats)["status"], "mixed");
        assert_eq!(
            review(&ranked, &[top, check("2", "t", true)])["status"],
            "mixed"
        );
        let mut tied = ranked;
        tied[1]["js_bits"] = tied[0]["js_bits"].clone();
        assert_eq!(review(&tied, &checks), json!({"status":"tied"}));
    }
}
