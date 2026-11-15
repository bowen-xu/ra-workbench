//! Resolve composer identities against frozen application sources, never renderer text.
use super::sources::MaterializationError;
use super::{Error, sources};
use crate::{
    agent::{writing_context::WritingContextReader, writing_resources::context_scope},
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    storage::AppStore,
};
use research_agent_core::{
    submission::ContextText,
    user_input::{MentionTarget, UserInput},
};
use rusqlite::Connection;
#[cfg(test)]
fn resolve(
    db: &Connection,
    store: &AppStore,
    input: &mut [UserInput],
    context: &[ContextText],
) -> Result<(), Error> {
    resolve_with_context(db, store, input, context, None).map_err(|error| match error {
        MaterializationError::Source(error) => error,
        MaterializationError::Writing(_) => Error::SourceUnavailable,
    })
}

pub(crate) fn resolve_with_context(
    db: &Connection,
    store: &AppStore,
    input: &mut [UserInput],
    context: &[ContextText],
    mut writing: Option<&mut dyn WritingContextReader>,
) -> Result<(), MaterializationError> {
    for item in input {
        if let UserInput::Mention {
            target:
                MentionTarget::File {
                    source_id,
                    resource_id,
                    source_url,
                    ..
                },
            ..
        } = item
            && let Some(encoded) = source_id.strip_prefix("context:")
        {
            let Ok(resource) = serde_json::from_str::<sources::Resource>(encoded) else {
                continue;
            };
            let resolved = (|| -> Result<ContextText, MaterializationError> {
                let grants = super::resource_grants::grants_with_scope(
                    db,
                    std::slice::from_ref(&resource),
                    context_scope(store.writing_resource_source(), writing.as_deref()),
                )?;
                let frozen = if let Some(frozen) = grants.first().and_then(|first| {
                    context
                        .iter()
                        .find(|source| source.source_id == first.source_id)
                }) {
                    frozen.clone()
                } else if let sources::Resource::Note { resource_id } = resource {
                    super::resource_grants::note_reference(db, context, resource_id)?
                        .ok_or(Error::SourceUnavailable)?
                } else {
                    let pieces = match writing.as_mut() {
                        Some(reader) => sources::materialize_with_context(
                            db,
                            store,
                            &[resource],
                            Some(&mut **reader),
                        ),
                        None => sources::materialize_with_context(db, store, &[resource], None),
                    }?;
                    let first = pieces.first().ok_or(Error::SourceUnavailable)?;
                    context
                        .iter()
                        .find(|source| source.source_id == first.source_id)
                        .ok_or(Error::ConfirmationMismatch)?
                        .clone()
                };
                Ok(frozen)
            })();
            let frozen = match resolved {
                Ok(value) => value,
                Err(MaterializationError::Source(
                    error @ (Error::Unavailable
                    | Error::Budget
                    | Error::NativeCatalogUnavailable(_)),
                )) => return Err(MaterializationError::Source(error)),
                // An unresolved file mention retains its original plain-text
                // identity. Technical source failures and pending Native reads
                // still leave preparation through the original SQL settlement.
                Err(MaterializationError::Writing(Flow::Fail(Failure {
                    cause: FailureCause::Original | FailureCause::NativeRejection(_),
                    ..
                }))) => continue,
                Err(MaterializationError::Writing(flow)) => {
                    return Err(MaterializationError::Writing(flow));
                }
                Err(_) => continue,
            };
            let (id, resource, url) = frozen_file_binding(&frozen);
            *source_id = id;
            *resource_id = resource;
            *source_url = url;
        }
    }
    Ok(())
}

/// Identical binding for first resolution and inspection of an immutable saved
/// source. This helper performs no current resource, file or permission lookup.
pub(crate) fn frozen_file_binding(source: &ContextText) -> (String, String, Option<String>) {
    (
        source.source_id.clone(),
        source.resource_id.clone(),
        super::resource_tools::source_link::address_for(
            super::resource_grants::resource(source).as_ref(),
            source.anchor.as_deref(),
        ),
    )
}

#[cfg(test)]
#[path = "references/tests.rs"]
mod tests;
