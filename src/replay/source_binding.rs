use super::Intervention;
use crate::{Call, ContextItem};

pub(super) fn resolve<'a>(
    call: &'a Call,
    intervention: &Intervention,
    source_id: Option<&str>,
    source_version: Option<&str>,
) -> Option<&'a ContextItem> {
    let matching = call.context.iter().filter(|item| {
        item.source_id.as_deref() == source_id && item.source_version.as_deref() == source_version
    });
    if let Some(item) = matching
        .clone()
        .find(|item| item.id == intervention.context_id)
    {
        return Some(item);
    }
    source_id?;
    // The importer may assign a different context ID to the same declared source.
    let mut candidates =
        matching.filter(
            |item| match (&item.request_fragment, intervention.text_range) {
                (Some(fragment), Some(range)) => {
                    fragment.pointer == intervention.pointer && fragment.text_range == range
                }
                (None, None) => true,
                _ => false,
            },
        );
    let item = candidates.next()?;
    candidates.next().is_none().then_some(item)
}
