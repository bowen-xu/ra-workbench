use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction};

use super::{
    error::NotesError,
    model::{NotePermanentDeleteReport, NotePermanentDeleteRequest},
    page_repository::sync_child_page_block,
};

fn verify_trashed(
    transaction: &Transaction<'_>,
    table: &str,
    scope_clause: &str,
    request: &NotePermanentDeleteRequest,
) -> Result<(), NotesError> {
    let sql = format!("select status, revision from {table} where id = ?1 {scope_clause}");
    let current = transaction
        .query_row(&sql, [&request.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()?
        .ok_or(NotesError::NotFound)?;
    if current.1 != request.expected_revision {
        return Err(NotesError::RevisionConflict {
            current_revision: current.1,
        });
    }
    if current.0 != "trashed" {
        return Err(NotesError::InvalidInput(
            "只有回收站中的笔记才能永久删除".to_owned(),
        ));
    }
    Ok(())
}

fn content_candidates(
    transaction: &Transaction<'_>,
    sql: &str,
    id: &str,
) -> Result<Vec<(String, i64)>, NotesError> {
    let mut statement = transaction.prepare(sql)?;
    Ok(statement
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn schedule_unreferenced_content(
    transaction: &Transaction<'_>,
    candidates: &[(String, i64)],
) -> Result<NotePermanentDeleteReport, NotesError> {
    let mut scheduled_asset_count = 0;
    let mut estimated_reclaimable_bytes = 0_i64;
    for (sha256, byte_size) in candidates {
        transaction.execute(
            "insert into content_object_gc_candidates (
                 content_sha256, unreferenced_since, not_before, reason
             )
             select sha256, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+30 days'),
                    'live_reference_removed'
             from content_objects
             where sha256 = ?1 and not exists (
                 select 1 from content_object_references where content_sha256 = ?1
             )
             on conflict(content_sha256) do nothing",
            [sha256],
        )?;
        let is_scheduled = transaction.query_row(
            "select exists(
               select 1 from content_object_gc_candidates g
               where g.content_sha256 = ?1
                 and not exists(
                   select 1 from content_object_references r
                   where r.content_sha256 = g.content_sha256
                 )
             )",
            [sha256],
            |row| row.get::<_, bool>(0),
        )?;
        if is_scheduled {
            scheduled_asset_count += 1;
            estimated_reclaimable_bytes = estimated_reclaimable_bytes.saturating_add(*byte_size);
        }
    }
    Ok(NotePermanentDeleteReport {
        scheduled_asset_count,
        estimated_reclaimable_bytes,
        cleanup_warnings: Vec::new(),
    })
}

pub(crate) fn permanently_delete_page_from(
    connection: &mut Connection,
    _content_directory: &Path,
    request: NotePermanentDeleteRequest,
) -> Result<NotePermanentDeleteReport, NotesError> {
    let transaction = connection.transaction()?;
    verify_trashed(&transaction, "note_pages", "", &request)?;
    let parent_id: Option<String> = transaction.query_row(
        "select parent_page_id from note_pages where id = ?1",
        [&request.id],
        |row| row.get(0),
    )?;
    let source_anchor: Option<String> = transaction
        .query_row(
            "select current_anchor_id from reading_note_sources where page_id=?1",
            [&request.id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    transaction.execute("delete from note_search where object_kind='reading_note' and object_id in(select block_id from reading_note_sources where page_id=?1)", [&request.id])?;
    let candidates = content_candidates(
        &transaction,
        "select distinct c.sha256, c.byte_size
         from note_blocks b join note_block_assets a on a.block_id = b.id
         join content_objects c on c.sha256 = a.content_sha256
         where b.page_id = ?1
         union select c.sha256,c.byte_size from content_objects c join reading_note_sources s on s.content_sha256=c.sha256 where s.page_id=?1
         union select c.sha256,c.byte_size from content_objects c join note_revision_assets a on a.content_sha256=c.sha256 join note_revisions r on r.id=a.revision_id where r.page_id=?1
         union select c.sha256,c.byte_size from content_objects c join hbs_note_baseline_assets a on a.content_sha256=c.sha256 where a.page_id=?1
         union select c.sha256,c.byte_size from content_objects c join hbs_preservation_assets a on a.content_sha256=c.sha256 where a.kind='note' and a.resource_id=?1",
        &request.id,
    )?;
    transaction.execute(
        "delete from note_search where object_kind = 'note_page' and object_id = ?1",
        [&request.id],
    )?;
    let descendants = {
        let mut statement = transaction.prepare(
            "with recursive subtree(id) as (
               select id from note_pages where parent_page_id = ?1
               union all select p.id from note_pages p join subtree s on p.parent_page_id = s.id
             ) select id from subtree",
        )?;
        statement
            .query_map([&request.id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    transaction.execute(
        "delete from note_links where source_block_id in (
           select id from note_blocks where block_type = 'childPage'
             and json_extract(content_json, '$.targetId') = ?1
         )",
        [&request.id],
    )?;
    transaction.execute(
        "delete from note_blocks where block_type = 'childPage'
         and json_extract(content_json, '$.targetId') = ?1",
        [&request.id],
    )?;
    if let Some(parent) = &parent_id {
        transaction.execute(
            "update note_pages set revision = revision + 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            [parent],
        )?;
    }
    transaction.execute(
        "update note_pages set parent_page_id = null, status = 'active', trashed_at = null,
           revision = revision + 1,
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where parent_page_id = ?1",
        [&request.id],
    )?;
    for descendant in &descendants {
        transaction.execute(
            "update note_pages set status = 'active', trashed_at = null,
             revision = revision + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1",
            [descendant],
        )?;
    }
    for descendant in &descendants {
        for parent in sync_child_page_block(&transaction, descendant)? {
            transaction.execute(
                "update note_pages set revision = revision + 1,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
                [parent],
            )?;
        }
    }
    // S-prime belongs to the page's lifetime, not to an editor window. Keep it
    // through trash/restore, then release it with the permanently deleted owner.
    transaction.execute(
        "delete from hbs_preservations where kind = 'note' and resource_id = ?1",
        [&request.id],
    )?;
    transaction.execute("delete from note_pages where id = ?1", [&request.id])?;
    if let Some(anchor) = source_anchor {
        transaction.execute("delete from source_anchors where id=?1 and not exists(select 1 from evidence_links where anchor_id=?1) and not exists(select 1 from annotations where anchor_id=?1) and not exists(select 1 from reading_note_sources where current_anchor_id=?1)", [anchor])?;
    }
    transaction.execute(
        "with ranked as (
           select id, row_number() over (order by position, id) - 1 as next_position
           from note_pages where parent_page_id is null
         )
         update note_pages set
           revision = revision + case when position <> (select next_position from ranked where ranked.id = note_pages.id) then 1 else 0 end,
           updated_at = case when position <> (select next_position from ranked where ranked.id = note_pages.id)
             then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else updated_at end,
           position = (select next_position from ranked where ranked.id = note_pages.id)
         where id in (select id from ranked)",
        [],
    )?;
    // HBS pins can be the only remaining reference; their release has no GC
    // trigger. Schedule only after all page-owned roots have been removed.
    let report = schedule_unreferenced_content(&transaction, &candidates)?;
    transaction.execute(
        "update note_index_state set state = 'dirty' where id = 1",
        [],
    )?;
    transaction.commit()?;
    Ok(report)
}

pub(crate) fn permanently_delete_reading_note_from(
    connection: &mut Connection,
    _content_directory: &Path,
    request: NotePermanentDeleteRequest,
) -> Result<NotePermanentDeleteReport, NotesError> {
    let note = super::reading_note_repository::reading_note_from(connection, &request.id)?;
    permanently_delete_page_from(
        connection,
        _content_directory,
        NotePermanentDeleteRequest {
            id: note.page_id,
            expected_revision: request.expected_revision,
        },
    )
}
