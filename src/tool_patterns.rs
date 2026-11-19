use crate::{
    context_links::{ContextLink, LinkKind},
    Call, Run,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct ToolPattern<'a> {
    pub organization_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub source: &'a str,
    pub tool: &'a str,
    pub run_count: usize,
    pub conversation_count: Option<usize>,
    pub turn_count: Option<usize>,
    pub linked_decisions: usize,
    pub argument_matches: usize,
    pub occurrences: Vec<ToolOccurrence<'a>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolOccurrence<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub decision_id: &'a str,
}

pub(crate) fn group<'a>(
    runs: impl Iterator<Item = &'a Run>,
    links: &[ContextLink<'a>],
) -> Vec<ToolPattern<'a>> {
    let linked: BTreeSet<_> = links
        .iter()
        .map(|link| (link.run_id, link.call_id, link.decision_id))
        .collect();
    let arguments: BTreeSet<_> = links
        .iter()
        .filter(|link| link.kind == LinkKind::ArgumentMatch)
        .map(|link| (link.run_id, link.call_id, link.decision_id))
        .collect();
    let mut groups = BTreeMap::new();
    for run in runs {
        let organization = run.organization_id.as_deref();
        let user = run.user_id.as_deref();
        let scopes = BTreeSet::from([(None, None), (organization, None), (organization, user)]);
        for call in &run.calls {
            for decision in call
                .decisions
                .iter()
                .filter(|decision| decision.kind == "tool_call")
            {
                let occurrence = ToolOccurrence {
                    run_id: &run.id,
                    call_id: &call.id,
                    decision_id: &decision.id,
                };
                for (organization, user) in &scopes {
                    groups
                        .entry((
                            *organization,
                            *user,
                            run.source.as_str(),
                            decision.label.as_str(),
                        ))
                        .or_insert_with(Vec::new)
                        .push((run, call, occurrence.clone()));
                }
            }
        }
    }

    let mut patterns: Vec<_> = groups
        .into_iter()
        .map(|((organization_id, user_id, source, tool), entries)| {
            let run_count = entries
                .iter()
                .map(|(run, _, _)| run.id.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            let conversations: Option<BTreeSet<_>> = entries
                .iter()
                .map(|(run, _, _)| conversation(run))
                .collect();
            let turns: Option<BTreeSet<_>> = entries
                .iter()
                .map(|(run, call, _)| turn(run, call))
                .collect();
            let linked_decisions = entries
                .iter()
                .filter(|(_, _, occurrence)| {
                    linked.contains(&(
                        occurrence.run_id,
                        occurrence.call_id,
                        occurrence.decision_id,
                    ))
                })
                .count();
            let argument_matches = entries
                .iter()
                .filter(|(_, _, occurrence)| {
                    arguments.contains(&(
                        occurrence.run_id,
                        occurrence.call_id,
                        occurrence.decision_id,
                    ))
                })
                .count();
            ToolPattern {
                organization_id,
                user_id,
                source,
                tool,
                run_count,
                conversation_count: conversations.map(|values| values.len()),
                turn_count: turns.map(|values| values.len()),
                linked_decisions,
                argument_matches,
                occurrences: entries
                    .into_iter()
                    .map(|(_, _, occurrence)| occurrence)
                    .collect(),
            }
        })
        .collect();
    patterns.sort_by_key(|pattern| std::cmp::Reverse(pattern.occurrences.len()));
    patterns
}

type Conversation<'a> = (Option<&'a str>, Option<&'a str>, &'a str);

fn conversation(run: &Run) -> Option<Conversation<'_>> {
    Some((
        run.organization_id.as_deref(),
        run.user_id.as_deref(),
        run.conversation_id.as_deref()?,
    ))
}

fn turn<'a>(run: &'a Run, call: &'a Call) -> Option<(Conversation<'a>, &'a str)> {
    Some((conversation(run)?, call.turn_id.as_deref()?))
}
