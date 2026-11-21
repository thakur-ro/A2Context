//! Locate literal destinations in a source-constrained reply and the messages for the same call.
use super::findings::{occurrence, Observation};
use a2context::{Call, Run};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

// Inspect conventional Markdown links on a numbered reference line. Nested destinations and
// reference-style Markdown need a different parser and are outside this check.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?im)^[ \t]*(?:[-*][ \t]+)?(?:source|reference)[ \t]+[0-9]{1,4}[ \t]*:[ \t]*\[([^\[\]\r\n]{1,160})\]\((https?://[A-Za-z0-9][^\s<>()\[\]]{1,2047})\)"
).unwrap()
});
static RESTRICTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:please )?(?:(?:provide an answer|answer(?: the question)?|respond) based (?:solely|only|exclusively) on|use (?:solely|only|exclusively)) (?:the )?(?:(?:provided|supplied|following|given) )?(?:sources|passages|documents|context)\b"
).unwrap()
});

pub(super) fn finding(run: &Run, call: &Call) -> Option<Observation> {
    let [reply] = call.decisions.as_slice() else {
        return None;
    };
    if reply.kind != "model_output" {
        return None;
    }
    let answer = reply.outcome.as_str()?;
    if !answer.contains("](") || answer.contains("```") || answer.contains("~~~") {
        return None;
    }
    let mut references = REFERENCE.captures_iter(answer).peekable();
    references.peek()?;
    let raw = call.recorded_request.as_ref()?;
    let body: Value = serde_json::from_str(&raw.body).ok()?;
    if [
        "tools",
        "functions",
        "web_search_options",
        "previous_response_id",
        "attachments",
        "modalities",
    ]
    .iter()
    .any(|key| {
        body.get(*key)
            .is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
    }) {
        return None;
    }
    let messages = body["messages"].as_array()?;
    let texts: Vec<_> = messages
        .iter()
        .map(|m| {
            if !matches!(
                m["role"].as_str(),
                Some("system" | "developer" | "user" | "assistant")
            ) || m.get("tool_calls").is_some()
                || m.get("function_call").is_some()
            {
                return None;
            }
            m["content"].as_str()
        })
        .collect::<Option<_>>()?;
    let (index, instruction) = texts
        .iter()
        .enumerate()
        .filter(|(i, _)| messages[*i]["role"] != "assistant")
        .find_map(|(i, text)| RESTRICTION.find(text).map(|m| (i, m)))?;
    let mut rows = Vec::new();
    let mut missing = false;
    for reference in references.take(16) {
        let dest = reference.get(2)?;
        let url = dest.as_str();
        let mut partial = false;
        let input = texts.iter().enumerate().find_map(|(i, text)| {
            text.match_indices(url).find_map(|(at, _)| {
                partial = true;
                let end = at + url.len();
                let begins = text[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| c.is_whitespace() || "(<".contains(c));
                let ends = text[end..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || ")>".contains(c));
                (begins && ends)
                    .then(|| json!({"pointer":format!("/messages/{i}/content"),"bytes":[at,end]}))
            })
        });
        // A literal inside a longer URL or ambiguous punctuation is insufficient evidence either way.
        if partial && input.is_none() {
            continue;
        }
        missing |= input.is_none();
        rows.push(json!({"label":reference.get(1)?.as_str(),"url":url,
            "output_bytes":[dest.start(),dest.end()],"input":input}));
    }
    missing.then(|| occurrence("reference_link",json!(["reference-link-v1",run.source,run.organization_id,call.model]),call,
        json!({"links":rows,"instruction":instruction.as_str(),"instruction_pointer":format!("/messages/{index}/content"),
            "instruction_bytes":[instruction.start(),instruction.end()],"messages_checked":texts.len(),"decision":reply.id,
            "request_evidence":raw.evidence,"reply_evidence":reply.evidence,"sources":[]})))
}
