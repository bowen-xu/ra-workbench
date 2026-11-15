//! Original context expansion with an explicit request-local writing reader.
use super::super::{catalog, docling_source, group_scope, structured};
use super::*;
use crate::agent::writing_context::ContextNeed;
use crate::{
    agent::writing_context::{WritingContextError, WritingContextReader},
    ai::structured_resource_validation::Flow,
};

pub(crate) enum MaterializationError {
    Source(Error),
    Writing(Flow<WritingContextError, ContextNeed>),
}
impl From<Error> for MaterializationError {
    fn from(error: Error) -> Self {
        Self::Source(error)
    }
}

pub fn materialize(
    db: &Connection,
    store: &AppStore,
    resources: &[Resource],
) -> Result<Vec<ContextText>, Error> {
    materialize_with_context(db, store, resources, None).map_err(|error| match error {
        MaterializationError::Source(error) => error,
        MaterializationError::Writing(_) => Error::SourceUnavailable,
    })
}

pub(crate) fn materialize_with_context(
    db: &Connection,
    store: &AppStore,
    resources: &[Resource],
    mut writing: Option<&mut dyn WritingContextReader>,
) -> Result<Vec<ContextText>, MaterializationError> {
    if resources.len() > 32 {
        return Err(Error::InvalidInput.into());
    }
    let mut selected = std::collections::HashSet::new();
    let mut context = Vec::new();
    for resource in resources {
        if !selected.insert(serde_json::to_string(resource).map_err(|_| Error::InvalidInput)?) {
            return Err(Error::InvalidInput.into());
        }
        match resource {
            Resource::PaperGroup {
                resource_id, parts, ..
            }
            | Resource::Manuscript {
                resource_id, parts, ..
            }
            | Resource::NoteGroup {
                resource_id, parts, ..
            } => {
                let current = {
                    let scope = crate::agent::writing_resources::context_scope(
                        store.writing_resource_source(),
                        writing.as_deref(),
                    );
                    catalog::validate_parts_with_scope(db, resource, resource_id, parts, scope)?;
                    group_scope::parts_with_scope(db, resource, scope)?
                };
                for part in current.iter().filter(|part| part.selected) {
                    let mut pieces = match writing.as_mut() {
                        Some(reader) => materialize_with_context(
                            db,
                            store,
                            std::slice::from_ref(&part.resource),
                            Some(&mut **reader),
                        ),
                        None => materialize_with_context(
                            db,
                            store,
                            std::slice::from_ref(&part.resource),
                            None,
                        ),
                    }?;
                    for source in &mut pieces {
                        let mut anchor = source
                            .anchor
                            .as_deref()
                            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                            .unwrap_or_else(|| serde_json::json!({}));
                        anchor["contextAccess"] =
                            serde_json::to_value(&part.access).map_err(|_| Error::InvalidInput)?;
                        anchor["contextOwner"] = serde_json::json!({"kind":match resource {Resource::PaperGroup {..}=>"paper",Resource::Manuscript {..}=>"manuscript",_=>"note"},"resourceId":resource_id});
                        source.anchor = Some(anchor.to_string());
                    }
                    context.extend(pieces);
                }
            }
            Resource::PaperNotes {
                resource_id,
                excluded_note_ids,
            } => {
                context.extend(catalog::reading_notes(
                    db,
                    store,
                    resource_id,
                    excluded_note_ids,
                )?);
            }
            Resource::PaperMetadata { resource_id } => context.push(catalog::metadata(
                db,
                resource_id,
                store
                    .paper_metadata_source()
                    .ok_or(Error::SourceUnavailable)?,
            )?),
            Resource::PaperPortrait { resource_id } => context.push(catalog::portrait(
                db,
                resource_id,
                store
                    .paper_metadata_source()
                    .ok_or(Error::SourceUnavailable)?,
                store
                    .paper_portrait_source()
                    .ok_or(Error::SourceUnavailable)?,
            )?),
            Resource::StructuredObject { resource_id, .. } => {
                let value = crate::structured_resources::read_text(db, resource_id, None, true)
                    .map_err(|_| Error::SourceUnavailable)?;
                context.push(catalog::source(resource_id, "structuredObject", value.text));
            }
            Resource::TemplateCatalog { resource_id, .. } => {
                if resource_id != structured::TEMPLATE_CATALOG_ID {
                    return Err(Error::InvalidInput.into());
                }
                context.push(catalog::source(resource_id, "templateCatalog", "Global reading template directory. Add File creates a private draft; publication requires explicit user confirmation.".into()));
            }
            Resource::Paper {
                resource_id,
                attachment_id,
                page_index,
            } => context.extend(docling_source::paper(
                db,
                resource_id,
                attachment_id.as_deref(),
                *page_index,
            )?),
            Resource::Note { resource_id } => {
                let revision = db
                    .query_row(
                        "select revision from note_pages where id=?1 and status='active'",
                        [resource_id.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::SourceUnavailable)?;
                context.push(
                    crate::notes::agent_context::materialize_in(
                        db,
                        &crate::notes::agent_context::NoteContextRequest {
                            page_id: *resource_id,
                            expected_revision: revision,
                            block_id: None,
                        },
                    )
                    .map_err(|_| Error::SourceUnavailable)?,
                );
            }
            Resource::ManuscriptFile { resource_id, path } => {
                let pieces = match writing.as_mut() {
                    Some(reader) => reader
                        .current_file(db, store.manuscript_directory(), *resource_id, path)
                        .map_err(MaterializationError::Writing)?,
                    None => store
                        .materialize_current_writing_file_in(db, *resource_id, path)
                        .map_err(|_| Error::SourceUnavailable)?,
                };
                context.extend(pieces);
            }
        }
        if context.len() > 256
            || context.iter().map(|s| s.text.len()).sum::<usize>() > 8 * 1024 * 1024
        {
            return Err(Error::Budget.into());
        }
    }
    // Whole-paper + page selections may overlap; each frozen source appears once.
    let mut identities = std::collections::HashSet::new();
    context.retain(|s| identities.insert(s.source_id.clone()));
    Ok(context)
}
