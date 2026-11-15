//! Frozen reading-note evidence from its ordinary document and retained PDF source.
//! This local preview does not grant disclosure or follow links into the library.
use super::agent_context::{NoteContextError as Error, NoteContextPreview};
use crate::storage::AppStore;
use research_agent_core::submission::{ContextSourceKind, ContextText};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_BYTES: i64 = 2 * 1024 * 1024;
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub reading_note_id: Uuid,
    pub expected_revision: i64,
}
pub(crate) fn validate(request: &Request) -> Result<(), Error> {
    if request.reading_note_id.is_nil()
        || !(0..=9_007_199_254_740_991).contains(&request.expected_revision)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
#[tauri::command]
pub async fn agent_preview_reading_note_context(
    store: tauri::State<'_, AppStore>,
    request: Request,
) -> Result<NoteContextPreview, Error> {
    Ok(NoteContextPreview {
        schema_version: 1,
        source: materialize(&store, request).await?,
    })
}
pub async fn materialize(store: &AppStore, request: Request) -> Result<ContextText, Error> {
    validate(&request)?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        materialize_in(&tx, &request)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
/// Caller retains one read snapshot through bounds checks and evidence assembly.
pub(crate) fn materialize_in(db: &Connection, request: &Request) -> Result<ContextText, Error> {
    validate(request)?;
    let id = request.reading_note_id.to_string();
    let (revision, status, bytes, schema_version): (i64, String, i64, i64) = db
        .query_row(
            "select b.revision,b.status,coalesce((select sum(length(cast(t.content_json as blob))+length(cast(t.props_json as blob))) from note_blocks t where t.page_id=b.id),0)+
         length(cast(s.paper_title_snapshot as blob))+length(cast(s.rects_json as blob))+
         length(cast(s.quote_exact as blob))+length(cast(s.quote_prefix as blob))+
         length(cast(s.quote_suffix as blob))+coalesce(length(cast(s.selector_json as blob)),0),b.schema_version
         from note_pages b join reading_note_sources s on s.page_id=b.id
         where s.block_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?
        .ok_or(Error::Unavailable)?;
    if status != "active" || schema_version != 1 {
        return Err(Error::Unavailable);
    }
    if revision != request.expected_revision {
        return Err(Error::Conflict);
    }
    if bytes > MAX_BYTES {
        return Err(Error::Budget);
    }
    let (kind,content,mut source):(String,String,Value)=db.query_row(
        "select s.note_type,b.id,s.current_anchor_id,s.paper_id_snapshot,s.paper_title_snapshot,
         s.attachment_id_snapshot,s.content_sha256,s.page_index,s.page_label,s.rects_json,
         s.quote_exact,s.quote_prefix,s.quote_suffix,s.selector_json,s.locator_version
         from note_pages b join reading_note_sources s on s.page_id=b.id where s.block_id=?1",[&id],|r|{
            Ok((r.get(0)?,r.get(1)?,json!({"anchorId":r.get::<_,Option<String>>(2)?,"paperId":r.get::<_,String>(3)?,
                "paperTitle":r.get::<_,String>(4)?,"attachmentId":r.get::<_,String>(5)?,"contentSha256":r.get::<_,String>(6)?,
                "pageIndex":r.get::<_,i64>(7)?,"pageLabel":r.get::<_,Option<String>>(8)?,"rects":r.get::<_,String>(9)?,
                "quoteExact":r.get::<_,String>(10)?,"quotePrefix":r.get::<_,String>(11)?,"quoteSuffix":r.get::<_,String>(12)?,
                "selector":r.get::<_,Option<String>>(13)?,"locatorVersion":r.get::<_,i64>(14)?})))
        }).map_err(|_|Error::Unavailable)?;
    let note_type = kind;
    // Share the ordinary evidence reader's bounds, depth and damaged-JSON checks.
    // This local read does not create an ordinary-note conversation or write grant.
    let document = super::agent_context::materialize_in(
        db,
        &super::agent_context::NoteContextRequest {
            page_id: Uuid::parse_str(&content).map_err(|_| Error::Unavailable)?,
            expected_revision: revision,
            block_id: None,
        },
    )?;
    let content: Value = serde_json::from_str(&document.text).map_err(|_| Error::Unavailable)?;
    let rects: Value = serde_json::from_str(source["rects"].as_str().ok_or(Error::Unavailable)?)
        .map_err(|_| Error::Unavailable)?;
    if !rects.is_array() {
        return Err(Error::Unavailable);
    }
    let geometry: Vec<super::model::SourceRect> =
        serde_json::from_value(rects.clone()).map_err(|_| Error::Unavailable)?;
    if geometry.iter().any(|rect| {
        !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
    }) {
        return Err(Error::Unavailable);
    }
    source["rects"] = rects;
    if let Some(raw) = source["selector"].as_str() {
        let selector: Value = serde_json::from_str(raw).map_err(|_| Error::Unavailable)?;
        if !selector.is_object() {
            return Err(Error::Unavailable);
        }
        let checked: super::model::SourceSelectorV2 =
            serde_json::from_value(selector.clone()).map_err(|_| Error::Unavailable)?;
        if !checked.is_valid_for(
            source["pageIndex"].as_i64().ok_or(Error::Unavailable)?,
            source["contentSha256"].as_str().ok_or(Error::Unavailable)?,
            source["quoteExact"].as_str().ok_or(Error::Unavailable)?,
        ) {
            return Err(Error::Unavailable);
        }
        source["selector"] = selector;
    }
    if source["locatorVersion"]
        != if source["selector"].is_null() {
            json!(1)
        } else {
            json!(2)
        }
    {
        return Err(Error::Unavailable);
    }
    if source["pageIndex"]
        .as_i64()
        .is_none_or(|page| !(0..=9_007_199_254_740_991).contains(&page))
        || source["contentSha256"].as_str().is_none_or(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(Error::Unavailable);
    }
    let anchor = json!({"schemaVersion":1,"readingNoteId":id,"noteRevision":revision,
        "sourceAnchorId":source["anchorId"],"paperId":source["paperId"],"attachmentId":source["attachmentId"],
        "contentSha256":source["contentSha256"],"pageIndex":source["pageIndex"],"locatorVersion":source["locatorVersion"]});
    let text=json!({"schemaVersion":1,"readingNoteId":id,"noteRevision":revision,"noteType":note_type,"content":content,"source":source}).to_string();
    if text.len() > MAX_BYTES as usize {
        return Err(Error::Budget);
    }
    Ok(ContextText {
        source_id: format!("reading-note:{}", request.reading_note_id),
        kind: ContextSourceKind::Note,
        resource_id: id,
        revision: format!(
            "revision:{revision}:sha256:{:x}",
            Sha256::digest(text.as_bytes())
        ),
        anchor: Some(anchor.to_string()),
        text,
    })
}

#[cfg(test)]
#[path = "reading_agent_context/tests.rs"]
mod tests;
