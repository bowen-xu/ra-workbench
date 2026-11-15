#[path = "document_repository/links.rs"]
mod links;
#[path = "document_repository/save.rs"]
mod save;
pub(crate) use save::save_document_from;
pub(crate) use save::save_document_in;
pub(crate) use save::save_preserving_in;

use std::collections::{HashMap, HashSet};

use links::replace_semantic_links;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    error::NotesError,
    model::{NoteDocument, NoteDocumentBlock, NoteDocumentSnapshot, NoteRevisionSummary},
    page_repository::page_from,
};

struct FlatBlock {
    id: String,
    parent_id: Option<String>,
    position: i64,
    block_type: String,
    props: serde_json::Value,
    content: serde_json::Value,
}

pub(crate) fn load_document_value(
    connection: &Connection,
    page_id: &str,
) -> Result<NoteDocument, NotesError> {
    let header =
        "select schema_version, coalesce(document_title,title) from note_pages where id = ?1";
    // Existing body readers are also used to prepare/inspect deployed databases
    // before migrations. Only the absent new projection falls back to metadata;
    // storage failures and every other malformed schema still propagate.
    let mut statement = match connection.prepare_cached(header) {
        Ok(statement) => Ok(statement),
        Err(rusqlite::Error::SqlInputError { ref msg, .. })
            if msg == "no such column: document_title" =>
        {
            connection.prepare_cached("select schema_version, title from note_pages where id = ?1")
        }
        Err(error) => Err(error),
    }?;
    let (schema_version, title) = statement
        .query_row([page_id], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))
        .optional()?
        .ok_or(NotesError::NotFound)?;
    let mut statement = connection.prepare(
        "select id, parent_block_id, position, block_type, props_json, content_json
         from note_blocks where page_id = ?1 and scope_kind = 'page' and status = 'active'
         order by parent_block_id, position, id",
    )?;
    let rows = statement
        .query_map([page_id], |row| {
            let props: String = row.get(4)?;
            let content: String = row.get(5)?;
            Ok(FlatBlock {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                position: row.get(2)?,
                block_type: row.get(3)?,
                props: serde_json::from_str(&props).unwrap_or_else(|_| serde_json::json!({})),
                content: serde_json::from_str(&content)
                    .unwrap_or_else(|_| serde_json::json!({"kind": "none"})),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let known: HashSet<String> = rows.iter().map(|row| row.id.clone()).collect();
    let mut grouped: HashMap<Option<String>, Vec<FlatBlock>> = HashMap::new();
    for row in rows {
        let key = row
            .parent_id
            .clone()
            .filter(|parent| known.contains(parent));
        grouped.entry(key).or_default().push(row);
    }
    for values in grouped.values_mut() {
        values.sort_by_key(|row| (row.position, row.id.clone()));
    }
    fn build(
        parent: Option<String>,
        grouped: &mut HashMap<Option<String>, Vec<FlatBlock>>,
    ) -> Vec<NoteDocumentBlock> {
        grouped
            .remove(&parent)
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                let children = build(Some(row.id.clone()), grouped);
                NoteDocumentBlock {
                    id: row.id,
                    block_type: row.block_type,
                    props: row.props,
                    content: row.content,
                    children,
                }
            })
            .collect()
    }
    Ok(NoteDocument {
        schema_version,
        page_id: page_id.to_owned(),
        title: Some(title),
        blocks: build(None, &mut grouped),
    })
}

pub(crate) fn load_document_from(
    connection: &Connection,
    page_id: &str,
) -> Result<NoteDocumentSnapshot, NotesError> {
    let page = page_from(connection, page_id)?;
    let document = load_document_value(connection, page_id)?;
    Ok(NoteDocumentSnapshot { page, document })
}

pub(super) fn plain_text(content: &serde_json::Value) -> String {
    match content.get("kind").and_then(serde_json::Value::as_str) {
        Some("formula") => content
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned(),
        Some("asset") => ["displayName", "caption"]
            .iter()
            .filter_map(|key| content.get(key).and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        Some("reference") => ["labelSnapshot", "quoteSnapshot"]
            .iter()
            .filter_map(|key| content.get(key).and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => {
            fn collect(value: &serde_json::Value, output: &mut Vec<String>) {
                if let Some(text) = value.get("text").and_then(serde_json::Value::as_str) {
                    output.push(text.to_owned());
                }
                if let Some(source) = value.get("source").and_then(serde_json::Value::as_str) {
                    output.push(source.to_owned());
                }
                if let Some(label) = value
                    .get("labelSnapshot")
                    .and_then(serde_json::Value::as_str)
                {
                    output.push(label.to_owned());
                }
                if let Some(items) = value.as_array() {
                    items.iter().for_each(|item| collect(item, output));
                }
                if let Some(object) = value.as_object() {
                    object.values().for_each(|item| collect(item, output));
                }
            }
            let mut output = Vec::new();
            collect(content, &mut output);
            output.join(" ")
        }
    }
}

pub(super) fn write_blocks(
    transaction: &Transaction<'_>,
    page_id: &str,
    blocks: &[NoteDocumentBlock],
    parent_id: Option<&str>,
    retained: &mut HashSet<String>,
) -> Result<(), NotesError> {
    for (position, block) in blocks.iter().enumerate() {
        retained.insert(block.id.clone());
        let changed = transaction.execute(
            "insert into note_blocks
             (id, scope_kind, page_id, parent_block_id, position, block_type, props_json, content_json,
              plain_text, schema_version, revision, status, is_favorite, created_at, updated_at)
             values (?1, 'page', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 0, 'active', 0,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(id) do update set page_id = excluded.page_id,
                 parent_block_id = excluded.parent_block_id, position = excluded.position,
                 block_type = excluded.block_type, props_json = excluded.props_json,
                 content_json = excluded.content_json, plain_text = excluded.plain_text,
                 status = 'active', trashed_at = null,
                 updated_at = excluded.updated_at
             where note_blocks.page_id is not excluded.page_id
                or note_blocks.parent_block_id is not excluded.parent_block_id
                or note_blocks.position <> excluded.position
                or note_blocks.block_type <> excluded.block_type
                or note_blocks.props_json <> excluded.props_json
                or note_blocks.content_json <> excluded.content_json
                or note_blocks.plain_text <> excluded.plain_text
                or note_blocks.status <> 'active'",
            params![block.id, page_id, parent_id, position as i64, block.block_type,
                block.props.to_string(), block.content.to_string(), plain_text(&block.content)],
        )?;
        let is_asset = block.content["kind"] == "asset";
        let asset_matches = !is_asset || transaction.query_row("select exists(select 1 from note_block_assets where block_id=?1 and content_sha256=?2 and display_name=?3 and mime_type=?4)", params![block.id, block.content["assetId"].as_str().unwrap_or(""),block.content["displayName"].as_str().unwrap_or("附件"),block.content["mimeType"].as_str().unwrap_or("application/octet-stream")], |row| row.get::<_,bool>(0))?;
        if changed == 0 && asset_matches {
            write_blocks(
                transaction,
                page_id,
                &block.children,
                Some(&block.id),
                retained,
            )?;
            continue;
        }
        if !is_asset {
            transaction.execute(
                "delete from note_block_assets where block_id = ?1",
                [&block.id],
            )?;
        }
        if block
            .content
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some("asset")
        {
            let asset_id = block
                .content
                .get("assetId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| NotesError::InvalidInput("附件块缺少资产 ID".to_owned()))?;
            let display_name = block
                .content
                .get("displayName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("附件");
            let mime_type = block
                .content
                .get("mimeType")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("application/octet-stream");
            transaction.execute(
                "delete from note_block_assets where block_id=?1 and content_sha256<>?2",
                params![block.id, asset_id],
            )?;
            transaction.execute(
                "insert into note_block_assets
                 (block_id, content_sha256, display_name, mime_type, created_at)
                 values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 on conflict(block_id,content_sha256) do update set display_name=excluded.display_name, mime_type=excluded.mime_type
                 where note_block_assets.content_sha256 <> excluded.content_sha256 or note_block_assets.display_name <> excluded.display_name or note_block_assets.mime_type <> excluded.mime_type",
                params![block.id, asset_id, display_name, mime_type],
            )?;
        }
        write_blocks(
            transaction,
            page_id,
            &block.children,
            Some(&block.id),
            retained,
        )?;
    }
    Ok(())
}

fn child_page_targets(blocks: &[NoteDocumentBlock]) -> Result<Vec<String>, NotesError> {
    fn visit(
        blocks: &[NoteDocumentBlock],
        depth: usize,
        targets: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) -> Result<(), NotesError> {
        for block in blocks {
            if block.block_type == "childPage" {
                if depth != 1 {
                    return Err(NotesError::InvalidInput(
                        "子页面块必须位于页面正文顶层".to_owned(),
                    ));
                }
                let target = block
                    .content
                    .get("targetId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if target.is_empty() || !seen.insert(target.to_owned()) {
                    return Err(NotesError::InvalidInput(
                        "子页面块目标为空或重复".to_owned(),
                    ));
                }
                targets.push(target.to_owned());
            }
            visit(&block.children, depth + 1, targets, seen)?;
        }
        Ok(())
    }
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    visit(blocks, 1, &mut targets, &mut seen)?;
    Ok(targets)
}

pub(crate) fn list_revisions_from(
    connection: &Connection,
    page_id: &str,
) -> Result<Vec<NoteRevisionSummary>, NotesError> {
    let mut statement = connection.prepare(
        "select id, page_id, page_revision, reason, created_at, is_pinned from note_revisions
         where page_id = ?1 order by page_revision desc limit 100",
    )?;
    Ok(statement
        .query_map([page_id], |row| {
            Ok(NoteRevisionSummary {
                id: row.get(0)?,
                page_id: row.get(1)?,
                page_revision: row.get(2)?,
                reason: row.get(3)?,
                created_at: row.get(4)?,
                is_pinned: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn set_revision_pinned_from(
    connection: &Connection,
    page_id: &str,
    revision_id: &str,
    is_pinned: bool,
) -> Result<NoteRevisionSummary, NotesError> {
    let changed = connection.execute(
        "update note_revisions set is_pinned = ?3 where id = ?1 and page_id = ?2",
        params![revision_id, page_id, is_pinned],
    )?;
    if changed == 0 {
        return Err(NotesError::NotFound);
    }
    connection
        .query_row(
            "select id, page_id, page_revision, reason, created_at, is_pinned
         from note_revisions where id = ?1 and page_id = ?2",
            params![revision_id, page_id],
            |row| {
                Ok(NoteRevisionSummary {
                    id: row.get(0)?,
                    page_id: row.get(1)?,
                    page_revision: row.get(2)?,
                    reason: row.get(3)?,
                    created_at: row.get(4)?,
                    is_pinned: row.get(5)?,
                })
            },
        )
        .map_err(Into::into)
}
