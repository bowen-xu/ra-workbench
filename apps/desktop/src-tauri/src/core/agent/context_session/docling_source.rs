//! PDF selections resolve to the exact-version Docling JSON, never the text index.
use super::Error;
use research_agent_core::submission::{ContextSourceKind, ContextText};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub(super) fn paper(
    db: &Connection,
    id: &str,
    attachment: Option<&str>,
    page: Option<u32>,
) -> Result<Vec<ContextText>, Error> {
    if id.trim().is_empty() || id.len() > 1024 {
        return Err(Error::InvalidInput);
    }
    let (attachment, hash, title): (String, String, String) = db.query_row("select a.id,a.content_sha256,i.title from attachments a join bibliographic_items i on i.id=a.item_id where i.id=?1 and i.trashed_at is null and a.attachment_kind='pdf' and (?2 is null or a.id=?2) order by a.created_at,a.id limit 1",params![id,attachment],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|Error::Unavailable)?.ok_or(Error::SourceUnavailable)?;
    let (run, status, document): (String, String, String) = db.query_row("select id,parse_status,json_content from docling_parse_runs where item_id=?1 and attachment_id=?2 and content_sha256=?3 and parse_status in ('success','partial_success') and json_content is not null order by finished_at desc,id desc limit 1",params![id,attachment,hash],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|Error::Unavailable)?.ok_or(Error::PaperNotParsed)?;
    if document.len() > 8 * 1024 * 1024 {
        return Err(Error::Budget);
    }
    let mut document: Value =
        serde_json::from_str(&document).map_err(|_| Error::SourceUnavailable)?;
    if let Some(page) = page {
        // Preserve source refs and provenance, excluding other pages' content.
        let mut scoped = json!({});
        for kind in ["texts", "tables", "pictures"] {
            let items = document[kind]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| {
                            item["prov"].as_array().is_some_and(|prov| {
                                prov.iter().any(|position| {
                                    position["page_no"].as_u64() == Some(u64::from(page) + 1)
                                })
                            })
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            scoped[kind] = json!(items);
        }
        if ["texts", "tables", "pictures"]
            .iter()
            .all(|kind| scoped[kind].as_array().is_none_or(Vec::is_empty))
        {
            return Err(Error::SourceUnavailable);
        }
        document = scoped;
    }
    let text = serde_json::to_string_pretty(&document).map_err(|_| Error::Unavailable)?;
    let revision = format!("docling:{run}");
    Ok(vec![ContextText {source_id:format!("paper:{attachment}:docling:{run}{}",page.map(|page|format!(":page:{page}")).unwrap_or_default()),kind:ContextSourceKind::Paper,resource_id:id.into(),revision,anchor:Some(json!({"attachmentId":attachment,"title":title,"content":"doclingJson","parseRunId":run,"parseStatus":status,"pageIndex":page}).to_string()),text}])
}
