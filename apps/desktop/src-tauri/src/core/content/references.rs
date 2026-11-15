use rusqlite::{Connection, params};

use super::{error::ContentError, model::ContentStorageReport};
use crate::content::gc_manager::ContentGcManager;

pub(crate) const MARK_BATCH_SIZE: usize = 10_000;

pub(crate) fn ensure_backfill_complete(connection: &Connection) -> Result<(), ContentError> {
    let (status, error): (String, Option<String>) = connection.query_row(
        "select backfill_status, last_error_code from content_gc_state where id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if status == "complete" {
        Ok(())
    } else {
        Err(ContentError::BackfillBlocked(error.unwrap_or(status)))
    }
}

pub(crate) fn has_references(
    connection: &Connection,
    content_sha256: &str,
) -> Result<bool, ContentError> {
    connection
        .query_row(
            "select exists(select 1 from content_object_references where content_sha256 = ?1)",
            [content_sha256],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn reconcile_candidates(connection: &mut Connection) -> Result<usize, ContentError> {
    ensure_backfill_complete(connection)?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "delete from content_object_gc_candidates
         where exists(select 1 from content_object_references r
                      where r.content_sha256 = content_object_gc_candidates.content_sha256)",
        [],
    )?;
    let inserted = transaction.execute(
        "insert into content_object_gc_candidates (
             content_sha256, unreferenced_since, not_before, reason
         )
         select c.sha256,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+30 days'),
                'reconciliation'
         from content_objects c
         where not exists(select 1 from content_object_references r where r.content_sha256 = c.sha256)
           and not exists(select 1 from content_object_gc_candidates g where g.content_sha256 = c.sha256)
         order by c.sha256 limit ?1",
        [MARK_BATCH_SIZE as i64],
    )?;
    transaction.execute(
        "update content_gc_state set last_mark_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = 1",
        [],
    )?;
    transaction.commit()?;
    Ok(inserted)
}

pub(crate) fn storage_report(
    connection: &Connection,
    manager: &ContentGcManager,
    available_bytes: u64,
    total_disk_bytes: u64,
) -> Result<ContentStorageReport, ContentError> {
    let (
        total_object_count,
        total_bytes,
        current_referenced_object_count,
        current_referenced_bytes,
        history_only_object_count,
        history_only_bytes,
        grace_object_count,
        grace_bytes,
        reclaimable_object_count,
        reclaimable_bytes,
    ): (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = connection.query_row(
        "with classified as (
           select c.sha256, c.byte_size,
             exists(select 1 from content_object_references r
                    where r.content_sha256 = c.sha256 and r.role <> 'history_asset') as has_current,
             exists(select 1 from content_object_references r
                    where r.content_sha256 = c.sha256 and r.role = 'history_asset') as has_history,
             (select not_before from content_object_gc_candidates g where g.content_sha256 = c.sha256) as not_before
           from content_objects c
         )
         select count(*), coalesce(sum(byte_size), 0),
           coalesce(sum(has_current), 0), coalesce(sum(case when has_current then byte_size else 0 end), 0),
           coalesce(sum(case when not has_current and has_history then 1 else 0 end), 0),
           coalesce(sum(case when not has_current and has_history then byte_size else 0 end), 0),
           coalesce(sum(case when not has_current and not has_history and julianday(not_before) > julianday('now') then 1 else 0 end), 0),
           coalesce(sum(case when not has_current and not has_history and julianday(not_before) > julianday('now') then byte_size else 0 end), 0),
           coalesce(sum(case when not has_current and not has_history and julianday(not_before) <= julianday('now') then 1 else 0 end), 0),
           coalesce(sum(case when not has_current and not has_history and julianday(not_before) <= julianday('now') then byte_size else 0 end), 0)
         from classified",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
    )?;
    let (pinned_history_object_count, pinned_history_bytes) = connection.query_row(
        "select count(*), coalesce(sum(c.byte_size), 0) from content_objects c
         where exists(
           select 1 from note_revision_assets a
           join note_revisions r on r.id = a.revision_id
           where a.content_sha256 = c.sha256 and r.is_pinned = 1
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (
        backfill_status,
        last_mark_at,
        last_sweep_at,
        last_success_count,
        last_success_bytes,
        last_error_code,
    ) = connection.query_row(
        "select backfill_status, last_mark_at, last_sweep_at, last_success_count,
                last_success_bytes, last_error_code from content_gc_state where id = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    Ok(ContentStorageReport {
        total_object_count,
        total_bytes,
        current_referenced_object_count,
        current_referenced_bytes,
        history_only_object_count,
        history_only_bytes,
        pinned_history_object_count,
        pinned_history_bytes,
        grace_object_count,
        grace_bytes,
        reclaimable_object_count,
        reclaimable_bytes,
        available_bytes,
        is_low_disk_space: available_bytes < 2 * 1024 * 1024 * 1024
            || available_bytes.saturating_mul(100) < total_disk_bytes.saturating_mul(5),
        backfill_status,
        gc_status: manager.status(),
        last_mark_at,
        last_sweep_at,
        last_success_count,
        last_success_bytes,
        last_error_code,
    })
}

pub(crate) fn record_candidate_error(
    connection: &Connection,
    content_sha256: &str,
    code: &str,
) -> Result<(), ContentError> {
    connection.execute(
        "update content_object_gc_candidates set attempt_count = attempt_count + 1,
             last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error_code = ?2,
             not_before = strftime('%Y-%m-%dT%H:%M:%fZ', 'now',
                 printf('+%d minutes', min(1440, 5 * (attempt_count + 1))))
         where content_sha256 = ?1",
        params![content_sha256, code],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    #[test]
    fn candidate_failures_use_bounded_backoff_without_leaking_paths() {
        let mut connection = Connection::open_in_memory().expect("database");
        migrate(&mut connection).expect("migrate");
        let sha256 = "c".repeat(64);
        connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', 1, 'now')",
            params![sha256, format!("cc/cc/{sha256}")],
        ).expect("object");

        record_candidate_error(&connection, &sha256, "hash_mismatch").expect("record failure");
        let (attempt_count, error_code, retry_minutes): (i64, String, f64) = connection
            .query_row(
                "select attempt_count, last_error_code,
                    (julianday(not_before) - julianday(last_attempt_at)) * 24 * 60
             from content_object_gc_candidates where content_sha256 = ?1",
                [&sha256],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("candidate");
        assert_eq!(attempt_count, 1);
        assert_eq!(error_code, "hash_mismatch");
        assert!((4.9..=10.1).contains(&retry_minutes));
    }
}
