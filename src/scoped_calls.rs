use crate::{
    activation::patterns::query::Thread,
    conflict_query::{timestamp, Scope, TimeWindow},
    Call, Run,
};
use serde::Deserialize;
use time::OffsetDateTime;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    #[serde(default)]
    scope: Scope,
    thread: Option<Thread>,
    after: Option<String>,
    before: Option<String>,
}

impl Selection {
    fn includes(&self, run: &Run) -> bool {
        self.scope.includes(run)
            && self
                .thread
                .as_ref()
                .is_none_or(|thread| thread.includes(run))
    }

    fn window(&self) -> Result<TimeWindow, String> {
        TimeWindow::parse(self.after.as_deref(), self.before.as_deref())
    }
}

pub(crate) struct ScopedCall<'a> {
    pub run: &'a Run,
    pub call: &'a Call,
    pub time: Option<OffsetDateTime>,
    pub primary: bool,
    pub comparison: bool,
}

pub(crate) fn visit<'a>(
    primary: &Selection,
    comparison: Option<&Selection>,
    runs: impl IntoIterator<Item = &'a Run>,
    mut visit: impl FnMut(ScopedCall<'a>) -> Result<(), String>,
) -> Result<(), String> {
    let primary_window = primary.window()?;
    let comparison_window = comparison.map(Selection::window).transpose()?;
    for run in runs {
        let primary_run = primary.includes(run);
        let comparison_run = comparison.is_some_and(|filter| filter.includes(run));
        if !primary_run && !comparison_run {
            continue;
        }
        for call in &run.calls {
            let time = call
                .started_at
                .as_deref()
                .and_then(|value| timestamp(value).ok());
            let primary = primary_run && primary_window.includes(time);
            let comparison = comparison_run
                && comparison_window
                    .as_ref()
                    .is_some_and(|window| window.includes(time));
            if primary || comparison {
                visit(ScopedCall {
                    run,
                    call,
                    time,
                    primary,
                    comparison,
                })?;
            }
        }
    }
    Ok(())
}

pub(crate) type ConversationKey<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str);

pub(crate) fn conversation_key(run: &Run) -> Option<ConversationKey<'_>> {
    Some((
        &run.source,
        run.organization_id.as_deref(),
        run.user_id.as_deref(),
        run.conversation_id.as_deref()?,
    ))
}

pub(crate) fn next_offset(offset: usize, limit: usize, count: usize) -> Option<usize> {
    let next = offset.saturating_add(limit);
    (next < count).then_some(next)
}
