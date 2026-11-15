//! Persist identities and permissions only; content is read when a tool is called.
use super::{
    Error, catalog,
    sources::{Access, Resource},
};
use research_agent_core::submission::{ContextSourceKind, ContextText};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) fn grants(
    db: &Connection,
    resources: &[Resource],
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<Vec<ContextText>, Error> {
    grants_with_scope(
        db,
        resources,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
}

pub(crate) fn grants_with_scope(
    db: &Connection,
    resources: &[Resource],
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<Vec<ContextText>, Error> {
    if resources.len() > 32 {
        return Err(Error::InvalidInput);
    }
    let mut output = Vec::new();
    for resource in resources {
        if let Resource::Note { resource_id } = resource {
            let mut covered = false;
            for group in resources {
                if super::group_scope::note_access(db, group, &resource_id.to_string())?.is_some() {
                    covered = true;
                    break;
                }
            }
            if covered {
                continue;
            }
        }
        match resource {
            Resource::PaperGroup {
                resource_id, parts, ..
            }
            | Resource::NoteGroup {
                resource_id, parts, ..
            }
            | Resource::Manuscript {
                resource_id, parts, ..
            } if !super::group_scope::is_dynamic(resource) => {
                catalog::validate_parts_with_scope(
                    db,
                    resource,
                    resource_id,
                    parts,
                    writing_source,
                )?;
                let effective = super::group_scope::parts_with_scope(db, resource, writing_source)?;
                for part in effective.iter().filter(|part| part.selected) {
                    let mut rows = grants_with_scope(
                        db,
                        std::slice::from_ref(&part.resource),
                        writing_source,
                    )?;
                    for row in &mut rows {
                        let mut anchor: Value =
                            serde_json::from_str(row.anchor.as_deref().ok_or(Error::InvalidInput)?)
                                .map_err(|_| Error::InvalidInput)?;
                        anchor["contextAccess"] = json!(part.access);
                        anchor["contextOwner"] = json!({"kind":match resource {Resource::PaperGroup {..}=>"paper",Resource::Manuscript {..}=>"manuscript",_=>"note"},"resourceId":resource_id});
                        row.anchor = Some(anchor.to_string());
                    }
                    output.extend(rows);
                }
            }
            _ => {
                let label = match label_with_scope(db, resource, writing_source) {
                    Ok(label) => label,
                    Err(Error::SourceUnavailable) => continue,
                    Err(error) => return Err(error),
                };
                let compact = super::group_scope::compact(resource);
                let resource = &compact;
                let encoded = serde_json::to_string(resource).map_err(|_| Error::InvalidInput)?;
                let hash = format!("{:x}", Sha256::digest(encoded.as_bytes()));
                let (kind, id) = match resource {
                    Resource::Note { resource_id } => {
                        (ContextSourceKind::Note, resource_id.to_string())
                    }
                    Resource::NoteGroup { resource_id, .. } => {
                        (ContextSourceKind::Note, resource_id.clone())
                    }
                    Resource::ManuscriptFile { path, .. } => {
                        (ContextSourceKind::Manuscript, path.clone())
                    }
                    Resource::Manuscript { resource_id, .. } => {
                        (ContextSourceKind::Manuscript, resource_id.clone())
                    }
                    Resource::PaperGroup { resource_id, .. }
                    | Resource::Paper { resource_id, .. }
                    | Resource::PaperNotes { resource_id, .. }
                    | Resource::PaperMetadata { resource_id }
                    | Resource::PaperPortrait { resource_id } => {
                        (ContextSourceKind::Paper, resource_id.clone())
                    }
                    Resource::StructuredObject { resource_id, .. }
                    | Resource::TemplateCatalog { resource_id, .. } => {
                        (ContextSourceKind::Paper, resource_id.clone())
                    }
                };
                let access = match resource {
                    Resource::PaperGroup {
                        new_member_access: Some(access),
                        ..
                    }
                    | Resource::Manuscript {
                        new_member_access: Some(access),
                        ..
                    }
                    | Resource::NoteGroup {
                        new_member_access: Some(access),
                        ..
                    } => access,
                    Resource::StructuredObject {
                        access: Some(access),
                        ..
                    }
                    | Resource::TemplateCatalog {
                        access: Some(access),
                        ..
                    } => access,
                    _ => &Access::ReadOnly,
                };
                let access =
                    if super::group_scope::group_access(resource) == Some(&Access::ReadOnly) {
                        &Access::ReadOnly
                    } else {
                        access
                    };
                let mut anchor =
                    json!({"content":"resourceGrant","resource":resource,"contextAccess":access});
                if let Resource::StructuredObject { resource_id, .. } = resource {
                    let object = crate::structured_resources::get_header(db, resource_id)
                        .map_err(|_| Error::SourceUnavailable)?;
                    anchor["sourceUrl"] = json!(object.source_uri);
                    anchor["resourceKind"] = json!(object.kind);
                }
                if let Resource::TemplateCatalog { .. } = resource {
                    anchor["sourceUrl"] =
                        json!("source://agents/skills/edit-structured-resources/assets/reading");
                }
                if let Resource::Manuscript { resource_id, .. } = resource {
                    anchor["manuscriptId"] = json!(resource_id);
                }
                if let Resource::ManuscriptFile { resource_id, path } = resource {
                    anchor["manuscriptId"] = json!(resource_id);
                    anchor["path"] = json!(path);
                }
                output.push(ContextText {source_id:format!("resource:{hash}"),kind,resource_id:id,revision:format!("grant:{hash}"),anchor:Some(anchor.to_string()),text:json!({"label":label,"readWith":"research_find_sources, research_list_sources, research_read_source, research_search_sources","content":"Available current resource; body not loaded."}).to_string()});
            }
        }
        if output.len() > 256 {
            return Err(Error::Budget);
        }
    }
    let mut seen = std::collections::HashSet::new();
    output.retain(|row| seen.insert(row.source_id.clone()));
    Ok(output)
}

pub(super) fn label(
    db: &Connection,
    resource: &Resource,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<String, Error> {
    label_with_scope(
        db,
        resource,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
}

pub(super) fn label_with_scope(
    db: &Connection,
    resource: &Resource,
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<String, Error> {
    if let Resource::StructuredObject { resource_id, .. } = resource {
        return super::structured::label(db, resource_id);
    }
    if let Resource::TemplateCatalog { resource_id, .. } = resource {
        return (resource_id == super::structured::TEMPLATE_CATALOG_ID)
            .then(|| "全局阅读模板（新建草稿）".into())
            .ok_or(Error::InvalidInput);
    }
    let result = match resource {
        Resource::Paper {resource_id,attachment_id,..} => db.query_row("select a.display_name from attachments a join bibliographic_items i on i.id=a.item_id where i.id=?1 and i.trashed_at is null and a.attachment_kind='pdf' and (?2 is null or a.id=?2) order by a.created_at,a.id limit 1",params![resource_id,attachment_id],|row|row.get::<_,String>(0)),
        Resource::PaperGroup {resource_id,..}|Resource::PaperMetadata {resource_id}|Resource::PaperPortrait {resource_id}|Resource::PaperNotes {resource_id,..} => db.query_row("select title from bibliographic_items where id=?1 and trashed_at is null",[resource_id],|row|row.get(0)),
        Resource::Note {resource_id}=>db.query_row("select title from note_pages where id=?1 and status='active'",[resource_id.to_string()],|row|row.get(0)),
        Resource::NoteGroup {resource_id, ..}=>db.query_row("select title from note_pages where id=?1 and status='active'",[resource_id],|row|row.get(0)),
        Resource::Manuscript {resource_id,..}=>db.query_row("select title from manuscripts where id=?1 and status='active'",[resource_id],|row|row.get(0)),
        Resource::ManuscriptFile {resource_id,path}=> {
            if path.starts_with('/') || path.contains('\\') || path.split('/').any(|part|part.is_empty()||part=="."||part=="..") {return Err(Error::InvalidInput);}
            db.query_row("select f.relative_path from manuscript_files f join manuscripts m on m.id=f.manuscript_id where m.id=?1 and m.status='active' and f.relative_path=?2 and f.file_kind in ('tex','bib','style')",params![resource_id.to_string(),path],|row|row.get(0))
        }
        Resource::StructuredObject { .. } | Resource::TemplateCatalog { .. } => unreachable!(),
    };
    if super::group_scope::is_dynamic(resource)
        && let Resource::PaperGroup {
            resource_id, parts, ..
        }
        | Resource::Manuscript {
            resource_id, parts, ..
        }
        | Resource::NoteGroup {
            resource_id, parts, ..
        } = resource
    {
        catalog::validate_parts_with_scope(db, resource, resource_id, parts, writing_source)?;
    }
    let label = result.map_err(|_| Error::SourceUnavailable)?;
    if let Resource::PaperNotes {
        resource_id,
        excluded_note_ids,
    } = resource
        && !catalog::valid_exclusions(db, resource_id, excluded_note_ids)?
    {
        return Err(Error::InvalidInput);
    }
    Ok(label)
}

pub(crate) fn resource(source: &ContextText) -> Option<Resource> {
    let anchor: Value = serde_json::from_str(source.anchor.as_deref()?).ok()?;
    if anchor["content"] != "resourceGrant" {
        return None;
    }
    serde_json::from_value(anchor["resource"].clone()).ok()
}

/// Resolve a member handle through its existing group authority, without adding
/// another grant or reading the note body. The same check is used at admission.
pub(crate) fn note_reference(
    db: &Connection,
    context: &[ContextText],
    page: uuid::Uuid,
) -> Result<Option<ContextText>, Error> {
    for source in context {
        let Some(group) = resource(source) else {
            continue;
        };
        let access = match &group {
            Resource::PaperNotes {
                resource_id,
                excluded_note_ids,
            } if !excluded_note_ids.contains(&page.to_string())
                && catalog::paper_note_pages(db, resource_id)?
                    .iter()
                    .any(|(id, _)| id == &page.to_string()) =>
            {
                let anchor: Value =
                    serde_json::from_str(source.anchor.as_deref().ok_or(Error::InvalidInput)?)
                        .map_err(|_| Error::InvalidInput)?;
                Some(
                    serde_json::from_value(anchor["contextAccess"].clone())
                        .map_err(|_| Error::InvalidInput)?,
                )
            }
            _ => super::group_scope::note_access(db, &group, &page.to_string())?,
        };
        let Some(access) = access else {
            continue;
        };
        // Resolving an exact note identity cannot consume a writing source.
        let mut identity = grants(db, &[Resource::Note { resource_id: page }], None)?
            .into_iter()
            .next()
            .ok_or(Error::SourceUnavailable)?;
        let mut anchor: Value =
            serde_json::from_str(identity.anchor.as_deref().ok_or(Error::InvalidInput)?)
                .map_err(|_| Error::InvalidInput)?;
        anchor["contextAccess"] = json!(access);
        identity.anchor = Some(anchor.to_string());
        return Ok(Some(identity));
    }
    Ok(None)
}
