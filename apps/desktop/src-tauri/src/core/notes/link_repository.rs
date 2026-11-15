use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{
    error::NotesError,
    model::{NoteLink, NoteLinkDraft, NoteLinkTargetOutput},
};

fn valid_target(kind: &str) -> bool {
    matches!(
        kind,
        "note_page" | "reading_note" | "paper" | "claim" | "manuscript"
    )
}

fn valid_relation(relation: &str) -> bool {
    matches!(relation, "reference" | "mention" | "derived_from")
}

fn target_exists(connection: &Connection, kind: &str, id: &str) -> Result<bool, NotesError> {
    let sql = match kind {
        "note_page" => "select exists(select 1 from note_pages where id = ?1)",
        "reading_note" => {
            "select exists(select 1 from reading_note_sources where block_id = ?1 or page_id = ?1)"
        }
        "paper" => "select exists(select 1 from bibliographic_items where id = ?1)",
        "claim" => "select exists(select 1 from claims where id = ?1)",
        "manuscript" => "select exists(select 1 from manuscripts where id = ?1)",
        _ => return Ok(false),
    };
    Ok(connection.query_row(sql, [id], |row| row.get(0))?)
}

fn link_from(connection: &Connection, id: &str) -> Result<NoteLink, NotesError> {
    connection
        .query_row(
            "select id, source_page_id, source_block_id, target_kind, target_id,
                relation_type, target_snapshot_json, created_at
         from note_links where id = ?1",
            [id],
            |row| {
                let target_kind: String = row.get(3)?;
                let target_id: String = row.get(4)?;
                let snapshot: serde_json::Value = serde_json::from_str(&row.get::<_, String>(6)?)
                    .unwrap_or_else(|_| serde_json::json!({}));
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    target_kind,
                    target_id,
                    row.get::<_, String>(5)?,
                    snapshot,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
        .map_or(
            Err(NotesError::NotFound),
            |(
                id,
                source_page_id,
                source_block_id,
                kind,
                target_id,
                relation,
                snapshot,
                created_at,
            )| {
                let is_broken = !target_exists(connection, &kind, &target_id)?;
                Ok(NoteLink {
                    id,
                    source_page_id,
                    source_block_id,
                    target: NoteLinkTargetOutput {
                        kind,
                        id: target_id,
                        label_snapshot: snapshot
                            .get("labelSnapshot")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        quote_snapshot: snapshot
                            .get("quoteSnapshot")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    },
                    relation,
                    is_broken,
                    created_at,
                })
            },
        )
}

pub(crate) fn upsert_link_from(
    connection: &Connection,
    draft: NoteLinkDraft,
) -> Result<NoteLink, NotesError> {
    if !valid_target(&draft.target.kind)
        || !valid_relation(&draft.relation)
        || draft.target.id.trim().is_empty()
    {
        return Err(NotesError::InvalidInput(
            "笔记链接类型或目标无效".to_owned(),
        ));
    }
    let source_exists: bool = connection.query_row(
        "select exists(select 1 from note_pages where id = ?1)",
        [&draft.source_page_id],
        |row| row.get(0),
    )?;
    if !source_exists || !target_exists(connection, &draft.target.kind, &draft.target.id)? {
        return Err(NotesError::NotFound);
    }
    if let Some(block_id) = &draft.source_block_id {
        let belongs: bool = connection.query_row(
            "select exists(select 1 from note_blocks where id = ?1 and page_id = ?2)",
            params![block_id, draft.source_page_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(NotesError::InvalidInput("链接来源块不属于页面".to_owned()));
        }
    }
    let snapshot = serde_json::json!({
        "labelSnapshot": draft.target.label_snapshot,
        "quoteSnapshot": draft.target.quote_snapshot,
    })
    .to_string();
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "insert into note_links
         (id, source_page_id, source_block_id, target_kind, target_id, relation_type, target_snapshot_json, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict do update set target_snapshot_json = excluded.target_snapshot_json",
        params![id, draft.source_page_id, draft.source_block_id, draft.target.kind, draft.target.id, draft.relation, snapshot],
    )?;
    let stored_id = connection.query_row(
        "select id from note_links where source_page_id = ?1
         and (source_block_id = ?2 or (source_block_id is null and ?2 is null))
         and target_kind = ?3 and target_id = ?4 and relation_type = ?5",
        params![
            draft.source_page_id,
            draft.source_block_id,
            draft.target.kind,
            draft.target.id,
            draft.relation
        ],
        |row| row.get::<_, String>(0),
    )?;
    link_from(connection, &stored_id)
}

pub(crate) fn remove_link_from(connection: &Connection, id: &str) -> Result<(), NotesError> {
    if connection.execute("delete from note_links where id = ?1", [id])? == 0 {
        return Err(NotesError::NotFound);
    }
    Ok(())
}

pub(crate) fn list_outgoing_from(
    connection: &Connection,
    page_id: &str,
) -> Result<Vec<NoteLink>, NotesError> {
    let mut statement = connection
        .prepare("select id from note_links where source_page_id = ?1 order by created_at")?;
    let ids = statement
        .query_map([page_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| link_from(connection, id)).collect()
}

pub(crate) fn list_backlinks_from(
    connection: &Connection,
    kind: &str,
    target_id: &str,
) -> Result<Vec<NoteLink>, NotesError> {
    if !valid_target(kind) {
        return Err(NotesError::InvalidInput("反向链接目标类型无效".to_owned()));
    }
    // A backlink belongs to its source page. Missing destinations are still
    // valid outgoing snapshots; only a missing source invalidates a backlink.
    // Existence is checked by SQLite, not inferred from a read/permission error
    // or the source's archived/trashed lifecycle state.
    connection.execute(
        "delete from note_links where target_kind = ?1 and target_id = ?2
         and not exists (select 1 from note_pages where id = note_links.source_page_id)",
        params![kind, target_id],
    )?;
    let mut statement = connection.prepare(
        "select id from note_links where target_kind = ?1 and target_id = ?2 order by created_at",
    )?;
    let ids = statement
        .query_map(params![kind, target_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| link_from(connection, id)).collect()
}

#[cfg(test)]
#[path = "link_repository_tests.rs"]
mod tests;
