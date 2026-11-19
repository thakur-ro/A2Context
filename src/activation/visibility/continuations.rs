//! Compare greedy outputs after reapplying measured source masks at each step.

use super::{query, Measurement, Report};
use crate::{
    activation::{
        generation_branches::{first_difference, Continuation, StopReason},
        ToolProposal,
    },
    turn_evidence,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct MaskContinuation {
    context_ids: Vec<String>,
    pub(super) start_output_token: usize,
    max_new_tokens: usize,
    clean: Continuation,
    masked: Continuation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call: Option<ToolProposal>,
}

pub(super) fn validate(
    report: &Report,
    measured: &BTreeMap<&str, &Measurement>,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for row in &report.continuations {
        let mut ids: Vec<_> = row.context_ids.iter().map(String::as_str).collect();
        ids.sort_unstable();
        let known_mask = match ids.as_slice() {
            [source] => measured.contains_key(source),
            [first, second] => report.pairs.iter().any(|pair| {
                let mut known = pair.context_ids.each_ref().map(String::as_str);
                known.sort_unstable();
                known == [*first, *second]
            }),
            _ => false,
        };
        let start = row.start_output_token;
        if !known_mask || start >= report.output_token_ids.len() || !seen.insert((start, ids)) {
            return Err(
                "a mask continuation needs a distinct measured source or pair and output start"
                    .into(),
            );
        }
        let prefix = &report.output_token_ids[..start];
        if row.clean.token_ids != report.output_token_ids
            || row.clean.text != report.baseline_answer
            || row.clean.stop_reason != StopReason::EndOfSequence
            || row.clean.steps(prefix, row.max_new_tokens).is_none()
        {
            return Err(
                "the all-visible continuation did not reproduce the recorded output".into(),
            );
        }
        if row.masked.steps(prefix, row.max_new_tokens).is_none()
            || (row.masked.token_ids == report.output_token_ids
                && row.masked.text != report.baseline_answer)
        {
            return Err("the masked continuation must keep the fixed prefix and record its stopping condition".into());
        }
        if row.tool_call.as_ref().is_some_and(|proposal| {
            report.baseline_tool_call.is_none()
                || row.masked.stop_reason != StopReason::EndOfSequence
                || proposal.name.trim().is_empty()
        }) || (report.baseline_tool_call.is_some()
            && row.masked.stop_reason == StopReason::EndOfSequence
            && row.masked.text == report.baseline_answer
            && row.tool_call != report.baseline_tool_call)
        {
            return Err(
                "a masked tool proposal must belong to a completed tool continuation".into(),
            );
        }
    }
    Ok(())
}

impl MaskContinuation {
    pub(super) fn matches(&self, ids: &[&str]) -> bool {
        self.context_ids.len() == ids.len()
            && self.context_ids.iter().all(|id| ids.contains(&id.as_str()))
    }

    pub(super) fn query_row(&self, report: &Report, index: usize, excerpts: bool) -> Value {
        let changed = first_difference(&report.output_token_ids, &self.masked.token_ids);
        let outcome = if self.masked.stop_reason == StopReason::TokenLimit {
            "token_limit"
        } else {
            match (&report.baseline_tool_call, &self.tool_call) {
                (Some(before), Some(after)) if before.name != after.name => "changed_tool",
                (Some(before), Some(after)) if before.arguments != after.arguments => {
                    "changed_arguments"
                }
                (Some(_), Some(_)) => "same_proposal",
                (Some(_), None) => "unparsed_proposal",
                (None, _) if changed.is_some() => "changed_output",
                _ => "same_output",
            }
        };
        let path = query::visibility_path(report, self.start_output_token, None);
        let mut row = json!({
            "context_ids": self.context_ids, "start_output_token": self.start_output_token,
            "max_new_tokens": self.max_new_tokens, "stop_reason": self.masked.stop_reason,
            "first_changed_output_token": changed, "total_output_tokens": self.masked.token_ids.len(),
            "outcome": outcome, "inspector_path": format!("{path}&continuation={index}"),
            "source_inspector_paths": self.context_ids.iter().map(|id| query::source_path(report, id)).collect::<Vec<_>>(),
        });
        if excerpts {
            row["text_excerpt"] = turn_evidence::excerpt(&self.masked.text);
            if let Some(proposal) = &self.tool_call {
                row["proposed_tool"] = turn_evidence::excerpt(&proposal.name);
            }
        }
        row
    }
}
