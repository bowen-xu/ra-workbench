//! Explicit application resource reads. No filesystem grants or recursive expansion.
use super::Error;
use crate::storage::AppStore;
use research_agent_core::submission::ContextText;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Resource {
    PaperGroup {
        #[serde(rename = "resourceId")]
        resource_id: String,
        parts: Vec<Part>,
        #[serde(
            rename = "newMemberAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        new_member_access: Option<Access>,
        #[serde(
            rename = "groupAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        group_access: Option<Access>,
    },
    Manuscript {
        #[serde(rename = "resourceId")]
        resource_id: String,
        parts: Vec<Part>,
        #[serde(
            rename = "newMemberAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        new_member_access: Option<Access>,
        #[serde(
            rename = "groupAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        group_access: Option<Access>,
    },
    NoteGroup {
        #[serde(rename = "resourceId")]
        resource_id: String,
        parts: Vec<Part>,
        #[serde(
            rename = "newMemberAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        new_member_access: Option<Access>,
        #[serde(
            rename = "groupAccess",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        group_access: Option<Access>,
    },
    PaperNotes {
        #[serde(rename = "resourceId")]
        resource_id: String,
        #[serde(rename = "excludedNoteIds")]
        excluded_note_ids: Vec<String>,
    },
    PaperMetadata {
        #[serde(rename = "resourceId")]
        resource_id: String,
    },
    PaperPortrait {
        #[serde(rename = "resourceId")]
        resource_id: String,
    },
    StructuredObject {
        #[serde(rename = "resourceId")]
        resource_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<Access>,
    },
    TemplateCatalog {
        #[serde(rename = "resourceId")]
        resource_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<Access>,
    },
    Paper {
        #[serde(rename = "resourceId")]
        resource_id: String,
        #[serde(
            rename = "attachmentId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        attachment_id: Option<String>,
        #[serde(rename = "pageIndex", default, skip_serializing_if = "Option::is_none")]
        page_index: Option<u32>,
    },
    Note {
        #[serde(rename = "resourceId")]
        resource_id: Uuid,
    },
    ManuscriptFile {
        #[serde(rename = "resourceId")]
        resource_id: Uuid,
        path: String,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    ReadOnly,
    ReadWrite,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Part {
    pub resource: Resource,
    pub label: String,
    pub selected: bool,
    pub access: Access,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub resource: Resource,
    pub label: String,
}

pub fn search(db: &Connection, query: &str) -> Result<Vec<Selection>, Error> {
    if query.len() > 512 {
        return Err(Error::InvalidInput);
    }
    // instr is literal: wildcard characters in user text have no special meaning.
    let mut found = Vec::new();
    let mut stmt = db.prepare("select i.id,a.id,i.title,a.display_name from bibliographic_items i join attachments a on a.item_id=i.id where i.trashed_at is null and a.attachment_kind='pdf' and instr(lower(i.title||' '||a.display_name),lower(?1))>0 order by i.updated_at desc,a.id limit 60").map_err(|_| Error::Unavailable)?;
    let rows = stmt
        .query_map([query], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| Error::Unavailable)?;
    for row in rows {
        let (id, attachment, title, name) = row.map_err(|_| Error::Unavailable)?;
        found.push(Selection {
            resource: Resource::Paper {
                resource_id: id,
                attachment_id: Some(attachment),
                page_index: None,
            },
            label: format!("{title} · {name}"),
        });
    }
    let mut stmt = db.prepare("select id,title from note_pages where status='active' and instr(lower(title),lower(?1))>0 order by updated_at desc limit 60").map_err(|_| Error::Unavailable)?;
    let rows = stmt
        .query_map([query], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| Error::Unavailable)?;
    for row in rows {
        let (id, label) = row.map_err(|_| Error::Unavailable)?;
        found.push(Selection {
            resource: Resource::Note {
                resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
            },
            label: format!("笔记 · {label}"),
        });
    }
    let mut stmt = db.prepare("select m.id,f.relative_path,m.title from manuscripts m join manuscript_files f on f.manuscript_id=m.id where m.status='active' and f.file_kind in ('tex','bib','style') and instr(lower(m.title||' '||f.relative_path),lower(?1))>0 order by m.updated_at desc,f.relative_path limit 60").map_err(|_| Error::Unavailable)?;
    let rows = stmt
        .query_map([query], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| Error::Unavailable)?;
    for row in rows {
        let (id, path, title) = row.map_err(|_| Error::Unavailable)?;
        found.push(Selection {
            label: format!("{title} · {path}"),
            resource: Resource::ManuscriptFile {
                resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
                path,
            },
        });
    }
    Ok(found)
}

#[path = "sources/materialization.rs"]
mod materialization;
pub use materialization::materialize;
pub(crate) use materialization::{MaterializationError, materialize_with_context};
