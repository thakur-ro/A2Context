//! Compare keys in an answer with labeled passages and citation examples in its input.
use super::findings::{occurrence, Observation};
use a2context::{Call, Run};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::LazyLock;

const ID: &str = r"[A-Za-z][A-Za-z0-9_-]{0,30}[-_][A-Fa-f0-9]{6,32}";
static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?m)^[ \t]*({ID}):[ \t]+\S")).unwrap());
static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"\b{ID}\b")).unwrap());
static CITATIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\({ID}(?:[ ,\t]+{ID})*\)")).unwrap());

fn family(key: &str) -> Option<(&str, usize)> {
    let i = key.rfind(['-', '_'])? + 1;
    Some((&key[..i], key.len() - i))
}

pub(super) fn finding(run: &Run, call: &Call) -> Option<Observation> {
    let [reply] = call.decisions.as_slice() else {
        return None;
    };
    if reply.kind != "model_output" {
        return None;
    }
    let answer = reply.outcome.as_str()?;
    let mut groups = CITATIONS.find_iter(answer).peekable();
    groups.peek()?;
    let body: Value = serde_json::from_str(&call.recorded_request.as_ref()?.body).ok()?;
    let messages = body["messages"].as_array()?;
    let user = messages.last()?;
    if user["role"] != "user" {
        return None;
    }
    let prompt = user["content"].as_str()?;
    let mut labels = BTreeSet::new();
    for m in messages {
        if !matches!(m["role"].as_str(), Some("user" | "system" | "developer")) {
            return None;
        }
        labels.extend(
            LABEL
                .captures_iter(m["content"].as_str()?)
                .filter_map(|c| c.get(1).map(|k| k.as_str())),
        );
    }
    if labels.len() < 2 {
        return None;
    }
    let mut examples = std::collections::BTreeMap::new();
    let mut in_examples = false;
    let mut offset = 0;
    for line in prompt.split_inclusive('\n') {
        if line.trim_start().starts_with('#') {
            let heading = line.to_ascii_lowercase();
            in_examples = heading.contains("citation example");
        } else if in_examples {
            for key in KEY.find_iter(line) {
                examples.entry(key.as_str()).or_insert(offset + key.start());
            }
        }
        offset += line.len();
    }
    for group in groups {
        for key in KEY.find_iter(group.as_str()) {
            let id = key.as_str();
            let Some(&in_prompt) = examples.get(id) else {
                continue;
            };
            if labels.contains(id) {
                continue;
            }
            let pattern = family(id)?;
            let peers: Vec<_> = labels
                .iter()
                .filter(|k| family(k) == Some(pattern))
                .collect();
            if peers.len() < 2 {
                continue;
            }
            let start = group.start() + key.start();
            return Some(occurrence(
                "example_citation",
                json!([
                    "example-citation-v1",
                    run.source,
                    run.organization_id,
                    call.model,
                    pattern
                ]),
                call,
                json!({"key":id,"pointer":format!("/messages/{}/content",messages.len()-1),
                    "input_bytes":[in_prompt,in_prompt+id.len()],"decision":reply.id,"reply_evidence":reply.evidence,
                    "output_bytes":[start,start+id.len()],"citation":group.as_str(),
                    "label_count":peers.len(),"label_examples":peers.iter().take(3).collect::<Vec<_>>(),"sources":[]}),
            ));
        }
    }
    None
}
