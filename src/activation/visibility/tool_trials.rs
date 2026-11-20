//! Proposal comparisons from validated masked decodes.

use super::continuations::Outcome;
use crate::{activation, replay::Choice};

pub struct ToolTrial<'a> {
    pub context_ids: Vec<&'a str>,
    pub outcome: &'static str,
    pub target: Option<&'a str>,
    pub target_tokens: usize,
    pub start: usize,
    pub fixed_prefix: &'a [u32],
    pub max_new_tokens: usize,
    pub original: Choice,
    pub masked: Option<Choice>,
    pub inspector_path: String,
    pub model_id: &'a str,
    pub model_revision: &'a str,
    pub runtime: &'a str,
    pub tokenizer_sha256: &'a str,
    pub log_probability_change: f64,
}

/// Summarize tool proposals after a source mask. The caller must validate the report
/// against the recorded decision before deriving observations.
pub fn tool_trials(report: &activation::Report) -> impl Iterator<Item = ToolTrial<'_>> {
    let report = match report {
        activation::Report::Visibility(report) => Some(report),
        _ => None,
    };
    report
        .into_iter()
        .filter_map(|report| Some((report, report.baseline_tool_call.as_ref()?)))
        .flat_map(|(report, original)| {
            report
                .continuations
                .iter()
                .enumerate()
                .filter_map(move |(index, branch)| {
                    let start = branch.start_output_token;
                    let ids = branch.context_ids();
                    let target = report
                        .targets
                        .iter()
                        .find(|target| target.token_range[0] == start);
                    let masked = match ids.as_slice() {
                        [id] => report
                            .sources
                            .iter()
                            .filter_map(|s| s.measured())
                            .find(|s| s.context_id == *id)?
                            .masked_log_probs
                            .as_slice(),
                        [a, b] => report
                            .pairs
                            .iter()
                            .find(|pair| {
                                let [x, y] = pair.context_ids.each_ref().map(String::as_str);
                                [x, y] == [*a, *b] || [y, x] == [*a, *b]
                            })?
                            .masked_log_probs
                            .as_slice(),
                        _ => return None,
                    };
                    let end = target.map_or(start + 1, |t| t.token_range[1]);
                    let log_probability_change = (start..end)
                        .map(|i| masked[i] - report.baseline_log_probs[i])
                        .sum::<f64>();
                    let outcome = match branch.outcome(report) {
                        Outcome::ChangedTool => "changed_tool",
                        Outcome::ChangedArguments => "changed_arguments",
                        Outcome::SameProposal => "same_proposal",
                        Outcome::UnparsedProposal => "unparsed_proposal",
                        Outcome::TokenLimit => "token_limit",
                        _ => return None,
                    };
                    let proposal = |tool: &super::ToolProposal| Choice {
                        kind: "tool_call".into(),
                        label: tool.name.clone(),
                        outcome: serde_json::Value::Object(tool.arguments.clone()),
                    };
                    let path = super::query::visibility_path(report, start, None);
                    Some(ToolTrial {
                        context_ids: ids,
                        outcome,
                        target: target.map(|t| t.key.as_str()),
                        target_tokens: end - start,
                        start,
                        fixed_prefix: &report.output_token_ids[..start],
                        max_new_tokens: branch.token_budget(),
                        original: proposal(original),
                        masked: branch.tool_proposal().map(proposal),
                        inspector_path: format!("{path}&continuation={index}"),
                        model_id: &report.model_id,
                        model_revision: &report.model_revision,
                        runtime: &report.runtime,
                        tokenizer_sha256: &report.tokenizer_sha256,
                        log_probability_change,
                    })
                })
        })
}
