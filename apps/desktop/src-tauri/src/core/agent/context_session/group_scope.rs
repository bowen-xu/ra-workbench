//! Durable object grants; inventory is resolved at tool time, never a body snapshot.
use super::{
    Error, catalog,
    sources::{Access, Part, Resource},
};
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

pub(crate) fn is_dynamic(resource: &Resource) -> bool {
    matches!(
        resource,
        Resource::PaperGroup {
            new_member_access: Some(_),
            ..
        } | Resource::Manuscript {
            new_member_access: Some(_),
            ..
        } | Resource::NoteGroup {
            new_member_access: Some(_),
            group_access: Some(_),
            ..
        }
    )
}

/// Drop inherited members and display labels; retain only explicit exceptions.
pub(crate) fn compact(resource: &Resource) -> Resource {
    let mut result = resource.clone();
    let (parts, access, paper) = match &mut result {
        Resource::PaperGroup {
            parts,
            new_member_access: Some(access),
            ..
        } => (parts, access, true),
        Resource::Manuscript {
            parts,
            new_member_access: Some(access),
            ..
        } => (parts, access, false),
        Resource::NoteGroup {
            parts,
            new_member_access: Some(access),
            ..
        } => (parts, access, false),
        _ => return result,
    };
    parts.retain(|part| {
        if paper && matches!(part.resource,Resource::Note {..}) && !part.selected {return false;}
        let inherited = if matches!(part.resource,Resource::Paper {..}) {&Access::ReadOnly} else {&*access};
        !part.selected || part.access != *inherited || paper && matches!(part.resource,Resource::Note {..})
            || matches!(&part.resource,Resource::PaperNotes {excluded_note_ids,..} if !excluded_note_ids.is_empty())
    });
    for part in parts {
        part.label = "权限覆盖".into();
    }
    result
}

pub(crate) fn parts(
    db: &Connection,
    resource: &Resource,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<Vec<Part>, Error> {
    parts_with_scope(
        db,
        resource,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
}

pub(crate) fn parts_with_scope(
    db: &Connection,
    resource: &Resource,
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<Vec<Part>, Error> {
    let (id, saved, default) = match resource {
        Resource::PaperGroup {
            resource_id,
            parts,
            new_member_access,
            ..
        }
        | Resource::Manuscript {
            resource_id,
            parts,
            new_member_access,
            ..
        }
        | Resource::NoteGroup {
            resource_id,
            parts,
            new_member_access,
            ..
        } => (resource_id, parts, new_member_access),
        _ => return Err(Error::InvalidInput),
    };
    catalog::validate_parts_with_scope(db, resource, id, saved, writing_source)?;
    let Some(access) = default else {
        let mut parts = saved.clone();
        apply_ceiling(resource, &mut parts);
        return Ok(parts);
    };
    let mut current = Vec::new();
    match resource {
        Resource::PaperGroup { .. } => {
            super::resource_grants::label_with_scope(
                db,
                &Resource::PaperMetadata {
                    resource_id: id.clone(),
                },
                writing_source,
            )?;
            current.push(member(
                Resource::PaperMetadata {
                    resource_id: id.clone(),
                },
                "元数据".into(),
                access,
            ));
            let mut stmt = db.prepare("select id,display_name from attachments where item_id=?1 and attachment_kind='pdf' order by created_at,id").map_err(|_|Error::Unavailable)?;
            let rows = stmt
                .query_map([id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| Error::Unavailable)?;
            for row in rows {
                let (attachment, name) = row.map_err(|_| Error::Unavailable)?;
                current.push(member(
                    Resource::Paper {
                        resource_id: id.clone(),
                        attachment_id: Some(attachment),
                        page_index: None,
                    },
                    name,
                    &Access::ReadOnly,
                ));
            }
            current.push(member(
                Resource::PaperNotes {
                    resource_id: id.clone(),
                    excluded_note_ids: vec![],
                },
                "阅读笔记".into(),
                access,
            ));
            current.extend(
                super::structured::paper_parts(db, id)?
                    .into_iter()
                    .map(|mut part| {
                        part.access = access.clone();
                        part
                    }),
            );
            // Preserve explicitly selected legacy reading-note leaves; scope owns new notes.
            for part in saved
                .iter()
                .filter(|part| matches!(part.resource, Resource::Note { .. }))
            {
                if super::resource_grants::label_with_scope(db, &part.resource, writing_source)
                    .is_ok()
                {
                    current.push(part.clone());
                }
            }
        }
        Resource::Manuscript { .. } => {
            let active: bool = db
                .query_row(
                    "select exists(select 1 from manuscripts where id=?1 and status='active')",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Unavailable)?;
            if !active {
                return Err(Error::SourceUnavailable);
            }
            let owner = Uuid::parse_str(id).map_err(|_| Error::InvalidInput)?;
            let source = writing_source.ok_or(Error::Unavailable)?;
            for path in source.list_text_paths(db, id)? {
                current.push(member(
                    Resource::ManuscriptFile {
                        resource_id: owner,
                        path: path.clone(),
                    },
                    path,
                    access,
                ));
            }
        }
        Resource::NoteGroup { .. } => {
            current.push(member(
                Resource::Note {
                    resource_id: Uuid::parse_str(id).map_err(|_| Error::InvalidInput)?,
                },
                "正文".into(),
                access,
            ));
        }
        _ => return Err(Error::InvalidInput),
    }
    for part in &mut current {
        if let Some(previous) = saved
            .iter()
            .find(|value| key(&value.resource) == key(&part.resource))
        {
            part.selected = previous.selected;
            part.access = previous.access.clone();
            part.resource = previous.resource.clone();
        } else if matches!(&part.resource, Resource::StructuredObject {resource_id,..} if crate::structured_resources::get_header(db, resource_id).is_ok_and(|resource| resource.kind == "portrait"))
            && let Some(previous) = saved.iter().find(|previous| matches!(&previous.resource, Resource::PaperPortrait {resource_id} if resource_id == id)) {
            part.selected = previous.selected;
            part.access = previous.access.clone();
        }
    }
    apply_ceiling(resource, &mut current);
    Ok(current)
}
pub(crate) fn group_access(resource: &Resource) -> Option<&Access> {
    match resource {
        Resource::PaperGroup { group_access, .. }
        | Resource::Manuscript { group_access, .. }
        | Resource::NoteGroup { group_access, .. } => group_access.as_ref(),
        _ => None,
    }
}
fn apply_ceiling(resource: &Resource, parts: &mut [Part]) {
    if group_access(resource) == Some(&Access::ReadOnly) {
        for part in parts {
            part.access = Access::ReadOnly;
        }
    }
}

/// An @ reference is a handle, not a second restriction on a selected note group.
/// Return None only when this group does not authorize the note at all.
pub(crate) fn note_access(
    db: &Connection,
    group: &Resource,
    page: &str,
) -> Result<Option<Access>, Error> {
    if !matches!(
        group,
        Resource::PaperGroup { .. } | Resource::NoteGroup { .. }
    ) {
        return Ok(None);
    }
    // The guard above permits only paper/note groups, never writing resources.
    let current = parts(db, group, None)?;
    if let Some(part)=current.iter().find(|part| part.selected && matches!(&part.resource,Resource::Note {resource_id} if resource_id.to_string()==page)) {
        return Ok(Some(part.access.clone()));
    }
    for part in current.iter().filter(|part| part.selected) {
        if let Resource::PaperNotes {
            resource_id,
            excluded_note_ids,
        } = &part.resource
        {
            if excluded_note_ids.iter().any(|id| id == page) {
                continue;
            }
            let owned:bool=db.query_row("select exists(select 1 from note_pages n where n.id=?1 and n.status='active' and (exists(select 1 from reading_note_sources s where s.page_id=n.id and s.paper_id_snapshot=?2) or exists(select 1 from note_links l where l.source_page_id=n.id and l.source_block_id is null and l.target_kind='paper' and l.target_id=?2 and l.relation_type='derived_from')))",rusqlite::params![page,resource_id],|row|row.get(0)).map_err(|_|Error::Unavailable)?;
            if owned {
                return Ok(Some(part.access.clone()));
            }
        }
    }
    Ok(None)
}
fn member(resource: Resource, label: String, access: &Access) -> Part {
    Part {
        resource,
        label,
        access: access.clone(),
        selected: true,
    }
}
fn key(resource: &Resource) -> String {
    match resource {
        Resource::PaperNotes { resource_id, .. } => format!("notes:{resource_id}"),
        Resource::StructuredObject { resource_id, .. } => format!("structured:{resource_id}"),
        Resource::TemplateCatalog { resource_id, .. } => format!("template-catalog:{resource_id}"),
        _ => serde_json::to_string(resource).unwrap_or_default(),
    }
}
/// Keep deleted overrides, but never accept an attachment belonging to another item.
pub(super) fn attachment_override(
    db: &Connection,
    id: &str,
    attachment: &str,
) -> Result<bool, Error> {
    let owner = db
        .query_row(
            "select item_id from attachments where id=?1",
            [attachment],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    Ok(owner.is_none_or(|owner| owner == id))
}
pub(crate) fn path_access(
    resource: &Resource,
    path: &str,
    write: bool,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> bool {
    path_access_with_scope(
        resource,
        path,
        write,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
    .unwrap_or(false)
}

pub(crate) fn path_access_with_scope(
    resource: &Resource,
    path: &str,
    write: bool,
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<bool, Error> {
    if write && group_access(resource) == Some(&Access::ReadOnly) {
        return Ok(false);
    }
    let Resource::Manuscript {
        parts,
        new_member_access,
        ..
    } = resource
    else {
        return Ok(false);
    };
    if research_agent_core::workspace::validate_path(path).is_err() {
        return Ok(false);
    }
    if let Some(part) = parts.iter().find(
        |part| matches!(&part.resource,Resource::ManuscriptFile {path:member,..} if member==path),
    ) {
        return Ok(part.selected && (!write || part.access == Access::ReadWrite));
    }
    // Text project scope does not authorize arbitrary binary/control files.
    let supported = match writing_source {
        Some(source) => source.is_text_path(path)?,
        None => false,
    };
    Ok(supported
        && new_member_access
            .as_ref()
            .is_some_and(|access| !write || *access == Access::ReadWrite))
}

#[cfg(test)]
#[path = "group_scope/tests.rs"]
mod tests;
