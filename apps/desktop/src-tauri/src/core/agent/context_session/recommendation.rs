//! Resolve visible identities through the existing dynamic resource directory.
//! No title search, catalog inventory, body read or permission grant occurs here.
use super::{
    Error, catalog, resource_grants,
    sources::{Access, Resource, Selection},
};
use crate::storage::AppStore;
use rusqlite::Connection;

pub(super) fn resolve(
    db: &Connection,
    candidate: Selection,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<Selection, Error> {
    if candidate.label.len() > 4096 {
        return Err(Error::InvalidInput);
    }
    let (resource_id, paper) = match &candidate.resource {
        Resource::Note { .. } => {
            resource_grants::label(db, &candidate.resource, writing_source)?;
            return catalog::recommend_note(db, candidate);
        }
        Resource::Paper { resource_id, .. } => (resource_id.clone(), true),
        Resource::ManuscriptFile { resource_id, .. } => (resource_id.to_string(), false),
        Resource::Manuscript { resource_id, .. } => (resource_id.clone(), false),
        _ => return Err(Error::InvalidInput),
    };
    // Validate the exact visible file or project before resolving group defaults.
    resource_grants::label(db, &candidate.resource, writing_source)?;
    let resource = if paper {
        Resource::PaperGroup {
            resource_id,
            parts: vec![],
            group_access: Some(Access::ReadWrite),
            new_member_access: Some(Access::ReadWrite),
        }
    } else {
        Resource::Manuscript {
            resource_id,
            parts: vec![],
            group_access: Some(Access::ReadWrite),
            new_member_access: Some(Access::ReadWrite),
        }
    };
    Ok(Selection {
        label: resource_grants::label(db, &resource, writing_source)?,
        resource,
    })
}

pub(crate) async fn recommend(store: &AppStore, candidate: Selection) -> Result<Selection, Error> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        resolve(&db, candidate, store.writing_resource_source())
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[cfg(test)]
#[path = "recommendation_tests.rs"]
mod tests;
