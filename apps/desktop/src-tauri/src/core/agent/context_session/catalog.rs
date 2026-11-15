//! Application-owned hierarchy. All child ownership is rechecked before freezing.
use super::{
    Error,
    sources::{Access, Part, Resource, Selection},
};
use research_agent_core::submission::{ContextSourceKind, ContextText};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn part(resource: Resource, label: String) -> Part {
    let access = if matches!(resource, Resource::Paper { .. }) {
        Access::ReadOnly
    } else {
        Access::ReadWrite
    };
    Part {
        resource,
        label,
        selected: true,
        access,
    }
}
fn pairs(db: &Connection, sql: &str, key: &str) -> Result<Vec<(String, String)>, Error> {
    let mut stmt = db.prepare(sql).map_err(|_| Error::Unavailable)?;
    stmt.query_map([key], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|_| Error::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::Unavailable)
}
/// Whole-paper notes and PDF annotations share the paper's note permission scope.
pub(super) fn paper_note_pages(
    db: &Connection,
    paper: &str,
) -> Result<Vec<(String, String)>, Error> {
    pairs(db, "select n.id,n.title from note_pages n where n.status='active' and (
        exists(select 1 from reading_note_sources s where s.page_id=n.id and s.paper_id_snapshot=?1)
        or exists(select 1 from note_links l where l.source_page_id=n.id and l.source_block_id is null
            and l.target_kind='paper' and l.target_id=?1 and l.relation_type='derived_from'))
        order by n.created_at,n.id", paper)
}

/// Resolve a visible note by identity. This is a UI suggestion only; preparing
/// a submission still validates ownership and access through the original path.
pub(super) fn recommend_note(db: &Connection, candidate: Selection) -> Result<Selection, Error> {
    let Resource::Note { resource_id } = &candidate.resource else {
        return Err(Error::InvalidInput);
    };
    let id = resource_id.to_string();
    let row = db.query_row(
        "select title,exists(select 1 from reading_note_sources where page_id=n.id) from note_pages n where id=?1 and status='active'",
        [&id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?)),
    ).optional().map_err(|_|Error::Unavailable)?;
    let Some((title, false)) = row else {
        return Ok(candidate);
    };
    Ok(Selection {
        label: format!("笔记 · {title}"),
        resource: Resource::NoteGroup {
            resource_id: id,
            new_member_access: Some(Access::ReadWrite),
            group_access: Some(Access::ReadWrite),
            parts: vec![part(candidate.resource, "正文".into())],
        },
    })
}
pub fn search(
    source: crate::agent::context_catalog::ContextCatalogSource,
    db: &Connection,
    query: &str,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<Vec<Selection>, Error> {
    search_with_papers(
        source,
        db,
        query,
        writing_source,
        source
            .search_papers
            .map(|_| &source as &dyn crate::agent::context_catalog::PaperCatalogReader),
    )
}
pub(super) fn search_with_papers(
    source: crate::agent::context_catalog::ContextCatalogSource,
    db: &Connection,
    query: &str,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    papers: Option<&dyn crate::agent::context_catalog::PaperCatalogReader>,
) -> Result<Vec<Selection>, Error> {
    let manuscripts = crate::agent::context_catalog::StaticManuscriptCatalog {
        catalog: source,
        resources: writing_source,
    };
    search_with_readers(
        db,
        query,
        papers,
        source
            .search_manuscripts
            .map(|_| &manuscripts as &dyn crate::agent::context_catalog::ManuscriptCatalogReader),
    )
}

pub(super) fn search_with_readers(
    db: &Connection,
    query: &str,
    papers: Option<&dyn crate::agent::context_catalog::PaperCatalogReader>,
    manuscripts: Option<&dyn crate::agent::context_catalog::ManuscriptCatalogReader>,
) -> Result<Vec<Selection>, Error> {
    if query.len() > 512 {
        return Err(Error::InvalidInput);
    }
    let mut rows = Vec::new();
    if let Some(papers) = papers {
        for (id, title) in papers.search_papers(db, query)? {
            let mut parts = vec![part(
                Resource::PaperMetadata {
                    resource_id: id.clone(),
                },
                "元数据".into(),
            )];
            for (attachment, name) in papers.list_pdf_attachments(db, &id)? {
                parts.push(part(
                    Resource::Paper {
                        resource_id: id.clone(),
                        attachment_id: Some(attachment),
                        page_index: None,
                    },
                    name,
                ));
            }
            parts.push(part(
                Resource::PaperNotes {
                    resource_id: id.clone(),
                    excluded_note_ids: vec![],
                },
                "阅读笔记".into(),
            ));
            for (page, label) in paper_note_pages(db, &id)? {
                let mut note = part(
                    Resource::Note {
                        resource_id: Uuid::parse_str(&page).map_err(|_| Error::Unavailable)?,
                    },
                    label,
                );
                note.selected = false; // Inventory for exclusions; the dynamic scope owns reads.
                parts.push(note);
            }
            parts.extend(super::structured::paper_parts(db, &id)?);
            rows.push(Selection {
                label: title,
                resource: Resource::PaperGroup {
                    new_member_access: Some(Access::ReadWrite),
                    group_access: Some(Access::ReadWrite),
                    resource_id: id,
                    parts,
                },
            });
        }
    }
    for (id, title) in pairs(
        db,
        "select id,title from note_pages n where status='active' and not exists(select 1 from reading_note_sources s where s.page_id=n.id) and instr(lower(title),lower(?1))>0 order by updated_at desc,id limit 60",
        query,
    )? {
        rows.push(Selection {
            label: format!("笔记 · {title}"),
            resource: Resource::NoteGroup {
                new_member_access: Some(Access::ReadWrite),
                group_access: Some(Access::ReadWrite),
                resource_id: id.clone(),
                parts: vec![part(
                    Resource::Note {
                        resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
                    },
                    "正文".into(),
                )],
            },
        });
    }
    if let Some(manuscripts) = manuscripts {
        for (id, title) in manuscripts.search_manuscripts(db, query)? {
            let mut parts = Vec::new();
            for path in manuscripts.list_text_paths(db, &id)? {
                parts.push(part(
                    Resource::ManuscriptFile {
                        resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
                        path: path.clone(),
                    },
                    path,
                ));
            }
            {
                rows.push(Selection {
                    label: title,
                    resource: Resource::Manuscript {
                        new_member_access: Some(Access::ReadWrite),
                        group_access: Some(Access::ReadWrite),
                        resource_id: id,
                        parts,
                    },
                });
            }
        }
    }
    if papers.is_some() {
        let catalog_label = "全局阅读模板（新建草稿）";
        if query.is_empty() || catalog_label.contains(query) {
            rows.push(Selection {
                label: catalog_label.into(),
                resource: Resource::TemplateCatalog {
                    resource_id: super::structured::TEMPLATE_CATALOG_ID.into(),
                    access: Some(Access::ReadOnly),
                },
            });
        }
        for template in crate::structured_resources::list_headers(db, None)
            .map_err(|_| Error::Unavailable)?
            .into_iter()
            .filter(|resource| {
                resource.kind == "readingTemplate"
                    && resource.paper_id.is_none()
                    && resource.revision > 0
                    && resource
                        .title
                        .to_lowercase()
                        .contains(&query.to_lowercase())
            })
        {
            rows.push(Selection {
                label: template.title,
                resource: Resource::StructuredObject {
                    resource_id: template.id,
                    access: Some(Access::ReadOnly),
                },
            });
        }
    }
    Ok(rows)
}
#[cfg(test)]
pub fn validate_parts(
    db: &Connection,
    owner: &Resource,
    id: &str,
    parts: &[Part],
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), Error> {
    validate_parts_with_scope(
        db,
        owner,
        id,
        parts,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
}

pub fn validate_parts_with_scope(
    db: &Connection,
    owner: &Resource,
    id: &str,
    parts: &[Part],
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<(), Error> {
    if (parts.is_empty() && !super::group_scope::is_dynamic(owner)) || parts.len() > 256 {
        return Err(Error::InvalidInput);
    }
    if matches!(
        owner,
        Resource::NoteGroup {
            new_member_access: Some(_),
            group_access: None,
            ..
        }
    ) {
        return Err(Error::InvalidInput);
    }
    let writing_source = if matches!(owner, Resource::Manuscript { .. }) {
        Some(writing_source.ok_or(Error::Unavailable)?.require()?)
    } else {
        writing_source
    };
    let mut keys = std::collections::HashSet::new();
    for part in parts {
        if part.label.trim().is_empty()
            || part.label.len() > 4096
            || !keys.insert(serde_json::to_string(&part.resource).map_err(|_| Error::InvalidInput)?)
        {
            return Err(Error::InvalidInput);
        }
        let valid = match (owner, &part.resource) {
            (Resource::PaperGroup {..}, Resource::Paper {resource_id, attachment_id:Some(attachment),page_index:None}) => resource_id == id && part.access == Access::ReadOnly && super::group_scope::attachment_override(db,id,attachment)?,
            (Resource::PaperGroup {..}, Resource::PaperNotes {resource_id, excluded_note_ids}) => resource_id == id && valid_exclusions(db, id, excluded_note_ids)?,
            (Resource::PaperGroup {..}, Resource::PaperMetadata {resource_id} | Resource::PaperPortrait {resource_id}) => resource_id == id,
            (Resource::PaperGroup {..}, Resource::StructuredObject {resource_id,..}) => super::structured::belongs(db, resource_id, id),
            (Resource::PaperGroup {..}, Resource::Note {resource_id}) => db.query_row("select exists(select 1 from reading_note_sources where page_id=?1 and paper_id_snapshot=?2) or exists(select 1 from note_links where source_page_id=?1 and source_block_id is null and target_kind='paper' and target_id=?2 and relation_type='derived_from') or (?3 and not exists(select 1 from reading_note_sources where page_id=?1))",params![resource_id.to_string(),id,!part.selected && super::group_scope::is_dynamic(owner)],|row|row.get::<_,bool>(0)).map_err(|_|Error::Unavailable)?,
            (Resource::Manuscript {..}, Resource::ManuscriptFile {resource_id,path}) => resource_id.to_string()==id && research_agent_core::workspace::validate_path(path).is_ok() && writing_source.ok_or(Error::Unavailable)?.is_text_path(path)?,
            (Resource::NoteGroup {..},Resource::Note {resource_id}) => resource_id.to_string()==id,
            _ => false,
        };
        if !valid {
            return Err(Error::InvalidInput);
        }
    }
    Ok(())
}
pub(super) fn source(id: &str, role: &str, text: String) -> ContextText {
    ContextText {
        source_id: format!("paper:{id}:{role}"),
        kind: ContextSourceKind::Paper,
        resource_id: id.into(),
        revision: format!("sha256:{:x}", Sha256::digest(text.as_bytes())),
        anchor: Some(json!({"content":role}).to_string()),
        text,
    }
}
pub fn metadata(
    db: &Connection,
    id: &str,
    metadata_source: crate::agent::paper_metadata::PaperMetadataSource,
) -> Result<ContextText, Error> {
    let text = metadata_source(db, id).map_err(|error| match error {
        crate::agent::paper_metadata::PaperMetadataError::SourceUnavailable => {
            Error::SourceUnavailable
        }
        crate::agent::paper_metadata::PaperMetadataError::Unavailable => Error::Unavailable,
    })?;
    Ok(source(id, "metadata", text))
}
pub fn portrait(
    db: &Connection,
    id: &str,
    metadata_source: crate::agent::paper_metadata::PaperMetadataSource,
    portrait_source: crate::agent::paper_portrait::PaperPortraitSource,
) -> Result<ContextText, Error> {
    // Complete metadata validation precedes every portrait read, including empty bodies.
    metadata(db, id, metadata_source)?;
    let text = portrait_source(db, id).map_err(|error| match error {
        crate::agent::paper_portrait::PaperPortraitError::Unavailable => Error::Unavailable,
        crate::agent::paper_portrait::PaperPortraitError::SourceUnavailable => {
            Error::SourceUnavailable
        }
    })?;
    Ok(source(id, "portrait", text))
}

pub(super) fn valid_exclusions(
    db: &Connection,
    paper: &str,
    ids: &[String],
) -> Result<bool, Error> {
    if ids.len() > 256
        || ids.iter().any(|id| Uuid::parse_str(id).is_err())
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        return Ok(false);
    }
    for id in ids {
        // Trashed notes can remain excluded; new notes are resolved on every turn.
        let owned: bool = db.query_row("select not exists(select 1 from reading_note_sources where page_id=?1 and paper_id_snapshot<>?2)
            and not exists(select 1 from note_links where source_page_id=?1 and source_block_id is null
                and target_kind='paper' and target_id<>?2 and relation_type='derived_from')",params![id,paper],|row|row.get(0)).map_err(|_|Error::Unavailable)?;
        if !owned {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn reading_notes(
    db: &Connection,
    store: &crate::storage::AppStore,
    paper: &str,
    excluded: &[String],
) -> Result<Vec<ContextText>, Error> {
    metadata(
        db,
        paper,
        store
            .paper_metadata_source()
            .ok_or(Error::SourceUnavailable)?,
    )?;
    if !valid_exclusions(db, paper, excluded)? {
        return Err(Error::InvalidInput);
    }
    let mut result = Vec::new();
    for (id, _) in paper_note_pages(db, paper)? {
        if excluded.contains(&id) {
            continue;
        }
        result.extend(super::sources::materialize(
            db,
            store,
            &[Resource::Note {
                resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
            }],
        )?);
        if result.len() > 256
            || result.iter().map(|source| source.text.len()).sum::<usize>() > 8 * 1024 * 1024
        {
            return Err(Error::Budget);
        }
    }
    Ok(result)
}
