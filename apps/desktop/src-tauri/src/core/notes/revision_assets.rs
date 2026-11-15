use std::collections::BTreeSet;

#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{Connection, Transaction, params};

use super::{
    document_validation::validate_document,
    error::NotesError,
    model::{NoteDocument, NoteDocumentBlock},
};

const BACKFILL_BATCH_SIZE: usize = 100;
const AUTOSAVE_RETENTION_DAYS: i64 = 14;
const IMPORTANT_RETENTION_DAYS: i64 = 30;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RevisionAssetBackfillProgress {
    Complete,
    Pending { processed: usize },
    Blocked { error_code: String },
}

pub(crate) fn asset_ids_from_document(
    document: &NoteDocument,
) -> Result<BTreeSet<String>, NotesError> {
    validate_document(document).map_err(NotesError::InvalidInput)?;
    fn visit(blocks: &[NoteDocumentBlock], output: &mut BTreeSet<String>) {
        for block in blocks {
            if block
                .content
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("asset")
                && let Some(id) = block
                    .content
                    .get("assetId")
                    .and_then(serde_json::Value::as_str)
            {
                output.insert(id.to_ascii_lowercase());
            }
            visit(&block.children, output);
        }
    }
    let mut output = BTreeSet::new();
    visit(&document.blocks, &mut output);
    Ok(output)
}

pub(crate) fn replace_revision_assets(
    transaction: &Transaction<'_>,
    revision_id: &str,
    document: &NoteDocument,
) -> Result<(), NotesError> {
    transaction.execute(
        "delete from note_revision_assets where revision_id = ?1",
        [revision_id],
    )?;
    for asset_id in asset_ids_from_document(document)? {
        transaction.execute(
            "insert into note_revision_assets (revision_id, content_sha256) values (?1, ?2)",
            params![revision_id, asset_id],
        )?;
    }
    Ok(())
}

fn mark_backfill_blocked(connection: &Connection, error_code: &str) -> Result<(), NotesError> {
    connection.execute(
        "update content_gc_state set backfill_status = 'blocked', job_state = 'failed',
             last_error_code = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = 1",
        [error_code],
    )?;
    Ok(())
}

pub(crate) fn backfill_revision_assets_batch(
    connection: &mut Connection,
) -> Result<RevisionAssetBackfillProgress, NotesError> {
    let (status, cursor): (String, Option<String>) = connection.query_row(
        "select backfill_status, backfill_cursor from content_gc_state where id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if status == "complete" {
        return Ok(RevisionAssetBackfillProgress::Complete);
    }
    if status == "blocked" {
        let error_code = connection
            .query_row(
                "select last_error_code from content_gc_state where id = 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )?
            .unwrap_or_else(|| "revision_asset_backfill_blocked".to_owned());
        return Ok(RevisionAssetBackfillProgress::Blocked { error_code });
    }

    connection.execute(
        "update content_gc_state set backfill_status = 'running', job_state = 'running',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = 1",
        [],
    )?;
    let revisions = {
        let mut statement = connection.prepare(
            "select id, document_json from note_revisions
             where ?1 is null or id > ?1 order by id limit ?2",
        )?;
        statement
            .query_map(params![cursor, BACKFILL_BATCH_SIZE as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    if revisions.is_empty() {
        connection.execute(
            "update content_gc_state set backfill_status = 'complete', job_state = 'idle',
                 backfill_cursor = null, last_error_code = null,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = 1",
            [],
        )?;
        return Ok(RevisionAssetBackfillProgress::Complete);
    }

    let transaction = connection.transaction()?;
    let mut last_id = None;
    for (revision_id, document_json) in &revisions {
        let document = match serde_json::from_str::<NoteDocument>(document_json) {
            Ok(value) => value,
            Err(_) => {
                drop(transaction);
                mark_backfill_blocked(connection, "revision_document_invalid_json")?;
                return Ok(RevisionAssetBackfillProgress::Blocked {
                    error_code: "revision_document_invalid_json".to_owned(),
                });
            }
        };
        if replace_revision_assets(&transaction, revision_id, &document).is_err() {
            drop(transaction);
            mark_backfill_blocked(connection, "revision_asset_reconciliation_failed")?;
            return Ok(RevisionAssetBackfillProgress::Blocked {
                error_code: "revision_asset_reconciliation_failed".to_owned(),
            });
        }
        last_id = Some(revision_id.clone());
    }
    transaction.execute(
        "update content_gc_state set backfill_cursor = ?1, backfill_status = 'pending',
             job_state = 'idle', last_error_code = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = 1",
        [last_id],
    )?;
    transaction.commit()?;
    Ok(RevisionAssetBackfillProgress::Pending {
        processed: revisions.len(),
    })
}

#[cfg(test)]
pub(crate) fn revision_asset_backfill_status(
    connection: &Connection,
) -> Result<String, NotesError> {
    connection
        .query_row(
            "select backfill_status from content_gc_state where id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(NotesError::NotFound)
}

fn prune_revisions_at(connection: &mut Connection, now: &str) -> Result<usize, NotesError> {
    let transaction = connection.transaction()?;
    let deleted = transaction.execute(
        "delete from note_revisions
         where is_pinned = 0
           and (
             (reason = 'autosave'
               and julianday(created_at) < julianday(?1, ?2))
             or
             (reason <> 'autosave'
               and julianday(created_at) < julianday(?1, ?3))
           )",
        params![
            now,
            format!("-{AUTOSAVE_RETENTION_DAYS} days"),
            format!("-{IMPORTANT_RETENTION_DAYS} days")
        ],
    )?;
    transaction.commit()?;
    Ok(deleted)
}

pub(crate) fn prune_revisions(connection: &mut Connection) -> Result<usize, NotesError> {
    let now = connection.query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get::<_, String>(0)
    })?;
    prune_revisions_at(connection, &now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn asset_document(page_id: &str, asset_id: &str) -> NoteDocument {
        NoteDocument {
            title: None,
            schema_version: 1,
            page_id: page_id.to_owned(),
            blocks: vec![NoteDocumentBlock {
                id: "asset-block".to_owned(),
                block_type: "image".to_owned(),
                props: serde_json::json!({}),
                content: serde_json::json!({
                    "kind": "asset",
                    "assetId": asset_id,
                    "displayName": "figure.png",
                    "mimeType": "image/png",
                    "caption": ""
                }),
                children: Vec::new(),
            }],
        }
    }

    #[test]
    fn backfills_revision_asset_references_before_gc_is_enabled() {
        let mut connection = Connection::open_in_memory().expect("open database");
        migrate(&mut connection).expect("migrate database");
        let asset_id = "a".repeat(64);
        connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', 1, 'now')",
            params![asset_id, format!("aa/aa/{asset_id}")],
        ).expect("asset");
        connection.execute(
            "insert into note_pages (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 1, 1, 'now', 'now')",
            [],
        ).expect("page");
        connection.execute(
            "insert into note_revisions (id, page_id, page_revision, schema_version, document_json, reason, created_at)
             values ('revision', 'page', 0, 1, ?1, 'manual', 'now')",
            [serde_json::to_string(&asset_document("page", &asset_id)).expect("json")],
        ).expect("revision");

        assert_eq!(
            backfill_revision_assets_batch(&mut connection).expect("batch"),
            RevisionAssetBackfillProgress::Pending { processed: 1 }
        );
        assert_eq!(
            backfill_revision_assets_batch(&mut connection).expect("complete"),
            RevisionAssetBackfillProgress::Complete
        );
        let count: i64 = connection.query_row(
            "select count(*) from note_revision_assets where revision_id = 'revision' and content_sha256 = ?1",
            [asset_id],
            |row| row.get(0),
        ).expect("relation");
        assert_eq!(count, 1);
        assert_eq!(
            revision_asset_backfill_status(&connection).expect("status"),
            "complete"
        );
    }

    #[test]
    fn invalid_revision_blocks_gc_fail_closed() {
        let mut connection = Connection::open_in_memory().expect("open database");
        migrate(&mut connection).expect("migrate database");
        connection.execute(
            "insert into note_pages (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 1, 1, 'now', 'now')",
            [],
        ).expect("page");
        connection.execute(
            "insert into note_revisions (id, page_id, page_revision, schema_version, document_json, reason, created_at)
             values ('revision', 'page', 0, 1, '{\"broken\":true}', 'manual', 'now')",
            [],
        ).expect("revision");

        assert_eq!(
            backfill_revision_assets_batch(&mut connection).expect("blocked"),
            RevisionAssetBackfillProgress::Blocked {
                error_code: "revision_document_invalid_json".to_owned()
            }
        );
        assert_eq!(
            revision_asset_backfill_status(&connection).expect("status"),
            "blocked"
        );
    }

    #[test]
    fn prunes_unpinned_revisions_using_confirmed_retention_windows() {
        let mut connection = Connection::open_in_memory().expect("open database");
        migrate(&mut connection).expect("migrate database");
        connection.execute(
            "insert into note_pages (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 1, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        ).expect("page");
        for (id, revision, reason, created_at, is_pinned) in [
            ("old-autosave", 1, "autosave", "2026-01-16T23:59:59Z", 0),
            ("kept-autosave", 2, "autosave", "2026-01-17T00:00:00Z", 0),
            ("old-important", 3, "manual", "2025-12-31T23:59:59Z", 0),
            ("kept-important", 4, "restore", "2026-01-01T00:00:00Z", 0),
            ("pinned-old", 5, "autosave", "2020-01-01T00:00:00Z", 1),
        ] {
            connection.execute(
                "insert into note_revisions
                 (id, page_id, page_revision, schema_version, document_json, reason, created_at, is_pinned)
                 values (?1, 'page', ?2, 1, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    revision,
                    serde_json::to_string(&asset_document("page", &"a".repeat(64))).expect("json"),
                    reason,
                    created_at,
                    is_pinned
                ],
            ).expect("revision");
        }

        assert_eq!(
            prune_revisions_at(&mut connection, "2026-01-31T00:00:00Z").expect("prune"),
            2
        );
        let remaining = connection
            .prepare("select id from note_revisions order by id")
            .expect("statement")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows");
        assert_eq!(
            remaining,
            vec!["kept-autosave", "kept-important", "pinned-old"]
        );
    }

    #[test]
    fn pruning_the_last_history_reference_schedules_a_thirty_day_candidate() {
        let mut connection = Connection::open_in_memory().expect("open database");
        migrate(&mut connection).expect("migrate database");
        let asset_id = "b".repeat(64);
        connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', 8, '2025-01-01T00:00:00Z')",
            params![asset_id, format!("bb/bb/{asset_id}")],
        ).expect("asset");
        connection.execute(
            "insert into note_pages (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 1, 1, '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z')",
            [],
        ).expect("page");
        connection
            .execute(
                "insert into note_revisions
             (id, page_id, page_revision, schema_version, document_json, reason, created_at)
             values ('revision', 'page', 0, 1, ?1, 'manual', '2025-01-01T00:00:00Z')",
                [serde_json::to_string(&asset_document("page", &asset_id)).expect("json")],
            )
            .expect("revision");
        connection
            .execute(
                "insert into note_revision_assets (revision_id, content_sha256)
             values ('revision', ?1)",
                [&asset_id],
            )
            .expect("revision asset");

        assert_eq!(
            prune_revisions_at(&mut connection, "2026-01-01T00:00:00Z").expect("prune"),
            1
        );
        let (reason, grace_days): (String, f64) = connection
            .query_row(
                "select reason, julianday(not_before) - julianday(unreferenced_since)
             from content_object_gc_candidates where content_sha256 = ?1",
                [&asset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("candidate");
        assert_eq!(reason, "revision_pruned");
        assert!((grace_days - 30.0).abs() < 0.001);
    }
}
