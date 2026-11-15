//! The existing source:// directory owns identities; adapters retain access checks.
use super::*;
use research_agent_core::text_patch::address;
use rusqlite::{Connection, OptionalExtension};

pub(super) enum Target {
    Note {
        page: String,
        parent: Option<String>,
    },
    File {
        project: String,
        path: String,
    },
    Paper(String),
    Structured(String),
    TemplateCatalog,
}
fn note_node(
    db: &Connection,
    record: &SubmissionRecord,
    id: &str,
    saved: bool,
) -> Result<Option<Target>, String> {
    if !saved
        && let Some(page) = note::pending(db, record.conversation_id)?
            .into_iter()
            .filter_map(|note| note.candidate.or(note.baseline))
            .find(|page| page.page_id == id)
    {
        return Ok(Some(Target::Note {
            page: page.page_id,
            parent: page.parent_id,
        }));
    }
    db.query_row(
        "select id,parent_page_id from note_pages where id=?1",
        [id],
        |row| {
            Ok(Target::Note {
                page: row.get(0)?,
                parent: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}
fn resolve_view(
    db: &Connection,
    record: &SubmissionRecord,
    source: &str,
    saved: bool,
) -> Result<Target, String> {
    let parts = address::parse(source)?;
    let root = parts.first().map(String::as_str).ok_or("找不到文件对象")?;
    let mut index_start = 2;
    let id = parts.get(1).ok_or("找不到文件对象")?;
    let mut node = match root {
        "notes" => {
            let page = db
                .query_row(
                    "select page_id from reading_note_sources where block_id=?1",
                    [id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            note_node(db, record, page.as_deref().unwrap_or(id), saved)?.ok_or("找不到文件对象")?
        }
        "writings" => {
            let exists: bool = db
                .query_row(
                    "select exists(select 1 from manuscripts where id=?1)",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !exists {
                return Err("找不到文件对象".into());
            }
            Target::File {
                project: id.clone(),
                path: String::new(),
            }
        }
        "library" => {
            let exists: bool = db.query_row("select exists(select 1 from bibliographic_items where id=?1 and trashed_at is null)", [id], |row| row.get(0)).map_err(|error| error.to_string())?;
            if !exists {
                return Err("找不到文件对象".into());
            }
            Target::Paper(id.clone())
        }
        "agents"
            if parts.get(1..5).is_some_and(|path| {
                path == ["skills", "edit-structured-resources", "assets", "reading"]
            }) =>
        {
            index_start = 5;
            Target::TemplateCatalog
        }
        _ => return Err("找不到文件对象".into()),
    };
    for (index, id) in parts.iter().enumerate().skip(index_start) {
        node = match node {
            Target::TemplateCatalog => {
                let template_id = id.strip_suffix(".json").ok_or("找不到文件对象")?;
                let resource = crate::structured_resources::get_header(db, template_id)
                    .map_err(|_| "template child is unavailable")?;
                if resource.kind != "readingTemplate" || resource.paper_id.is_some() {
                    return Err("template does not belong to this catalog".into());
                }
                Target::Structured(template_id.to_owned())
            }
            Target::Structured(_) => return Err("structured object has no child path".into()),
            Target::File { project, .. } => {
                let path = parts[index..].join("/");
                research_agent_core::workspace::validate_path(&path)
                    .map_err(|_| "invalid project file identity")?;
                return Ok(Target::File { project, path });
            }
            Target::Note { .. } => return Err("找不到文件对象".into()),
            Target::Paper(paper) => {
                if let Ok(resource) = crate::structured_resources::get_header(db, id) {
                    if resource.paper_id.as_deref() != Some(&paper) {
                        return Err("structured object belongs to another paper".into());
                    }
                    node = Target::Structured(id.clone());
                    continue;
                }
                return Err("找不到文件对象".into());
            }
        };
    }
    Ok(node)
}
pub(super) fn resolve(
    db: &Connection,
    record: &SubmissionRecord,
    source: &str,
) -> Result<Target, String> {
    resolve_view(db, record, source, false)
}
pub(super) fn resolve_saved(
    db: &Connection,
    record: &SubmissionRecord,
    source: &str,
) -> Result<Target, String> {
    resolve_view(db, record, source, true)
}
/// Namespace classification is pure; it must not probe unrelated catalogs.
pub(super) fn is_note(path: &str) -> bool {
    path.starts_with("notes/") || address::parse(path).is_ok_and(|parts| parts[0] == "notes")
}
