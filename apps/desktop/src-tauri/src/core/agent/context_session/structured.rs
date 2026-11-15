//! Structured research objects share the source directory and resource ceilings.
use super::{
    Error, resource_grants,
    sources::{Access, Part, Resource},
};
use research_agent_core::submission::SubmissionRecord;
use rusqlite::Connection;
use serde_json::Value;

pub(crate) const TEMPLATE_CATALOG_ID: &str = crate::structured_resources::TEMPLATE_CATALOG_ID;

pub(super) fn label(db: &Connection, id: &str) -> Result<String, Error> {
    crate::structured_resources::get_header(db, id)
        .map(|resource| resource.title)
        .map_err(|_| Error::SourceUnavailable)
}

pub(super) fn paper_parts(db: &Connection, paper: &str) -> Result<Vec<Part>, Error> {
    crate::structured_resources::list_headers(db, Some(paper))
        .map_err(|_| Error::Unavailable)?
        .into_iter()
        .map(|resource| {
            Ok(Part {
                resource: Resource::StructuredObject {
                    resource_id: resource.id,
                    access: None,
                },
                label: resource.title,
                selected: true,
                access: Access::ReadWrite,
            })
        })
        .collect()
}

pub(super) fn belongs(db: &Connection, id: &str, paper: &str) -> bool {
    crate::structured_resources::get_header(db, id)
        .is_ok_and(|resource| resource.paper_id.as_deref() == Some(paper))
}

pub(crate) fn access(
    db: &Connection,
    record: &SubmissionRecord,
    id: &str,
    write: bool,
) -> Result<bool, String> {
    for source in &record.input.context {
        let Some(resource) = resource_grants::resource(source) else {
            continue;
        };
        let anchor: Value =
            serde_json::from_str(source.anchor.as_deref().ok_or("missing resource grant")?)
                .map_err(|error| error.to_string())?;
        let direct = match &resource {
            Resource::StructuredObject { resource_id, .. }
            | Resource::TemplateCatalog { resource_id, .. } => resource_id == id,
            Resource::PaperPortrait { resource_id } => {
                crate::structured_resources::get_header(db, id).is_ok_and(|resource| {
                    resource.kind == "portrait" && resource.paper_id.as_deref() == Some(resource_id)
                })
            }
            _ => false,
        };
        if direct && (!write || anchor["contextAccess"] == "readWrite") {
            return Ok(true);
        }
        // This branch resolves paper-owned objects only; it cannot query writing.
        if matches!(resource, Resource::PaperGroup { .. }) {
            let parts = super::group_scope::parts(db, &resource, None)
                .map_err(|error| error.to_string())?;
            if parts.iter().any(|part| part.selected && (!write || part.access == Access::ReadWrite) && matches!(&part.resource, Resource::StructuredObject { resource_id, .. } if resource_id == id)) { return Ok(true); }
        }
    }
    if id != TEMPLATE_CATALOG_ID && access_catalog(record, write) {
        if !write
            && crate::structured_resources::get_header(db, id).is_ok_and(|resource| {
                resource.kind == "readingTemplate"
                    && resource.paper_id.is_none()
                    && resource.revision > 0
            })
        {
            return Ok(true);
        }
        let owned = crate::structured_resources::get_header(db, id).is_ok_and(|resource| {
            resource.kind == "readingTemplate" && resource.paper_id.is_none()
        }) && crate::structured_resources::has_owned_draft(
            db,
            id,
            &record.conversation_id.to_string(),
        )
        .map_err(|error| error.to_string())?;
        if owned {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn access_catalog(record: &SubmissionRecord, write: bool) -> bool {
    record.input.context.iter().any(|source| {
        matches!(resource_grants::resource(source), Some(Resource::TemplateCatalog { resource_id, .. }) if resource_id == TEMPLATE_CATALOG_ID)
            && (!write || source.anchor.as_deref().and_then(|anchor| serde_json::from_str::<Value>(anchor).ok()).is_some_and(|anchor| anchor["contextAccess"] == "readWrite"))
    })
}
