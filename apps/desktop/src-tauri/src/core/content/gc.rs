use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::{
    error::ContentError,
    gc_manager::ContentGcManager,
    model::ContentGcRunReport,
    references::{
        MARK_BATCH_SIZE, ensure_backfill_complete, has_references, reconcile_candidates,
        record_candidate_error,
    },
};
use crate::{notes::revision_assets, storage::AppStore};

const IO_CHUNK_BYTES: usize = 64 * 1024;
const SWEEP_OBJECT_LIMIT: usize = 100;
const SWEEP_BYTE_LIMIT: u64 = 256 * 1024 * 1024;

fn expected_relative_path(sha256: &str) -> Result<PathBuf, ContentError> {
    if sha256.len() != 64
        || !sha256.bytes().all(|value| value.is_ascii_hexdigit())
        || sha256.bytes().any(|value| value.is_ascii_uppercase())
    {
        return Err(ContentError::UnsafePath);
    }
    Ok(PathBuf::from(&sha256[..2]).join(&sha256[2..4]).join(sha256))
}

fn ensure_not_symlink(path: &Path) -> Result<(), ContentError> {
    if path.exists() && fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(ContentError::UnsafePath);
    }
    Ok(())
}

fn validate_object_path(
    content_directory: &Path,
    sha256: &str,
    stored_relative_path: &str,
) -> Result<PathBuf, ContentError> {
    ensure_not_symlink(content_directory)?;
    let expected = expected_relative_path(sha256)?;
    if Path::new(stored_relative_path) != expected {
        return Err(ContentError::UnsafePath);
    }
    let first = content_directory.join(&sha256[..2]);
    let second = first.join(&sha256[2..4]);
    let object = content_directory.join(expected);
    ensure_not_symlink(&first)?;
    ensure_not_symlink(&second)?;
    ensure_not_symlink(&object)?;
    Ok(object)
}

fn hash_file(path: &Path, manager: &ContentGcManager) -> Result<(String, u64), ContentError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    let mut buffer = vec![0_u8; IO_CHUNK_BYTES];
    loop {
        manager.check_cancelled()?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        byte_size = byte_size.saturating_add(read as u64);
    }
    Ok((format!("{:x}", hasher.finalize()), byte_size))
}

pub(crate) fn recover_quarantine(
    store: &AppStore,
    manager: &ContentGcManager,
) -> Result<Vec<String>, ContentError> {
    let _content_write = store.content_write()?;
    let quarantine = store.content_directory().join(".gc");
    if !quarantine.exists() {
        return Ok(Vec::new());
    }
    ensure_not_symlink(&quarantine)?;
    let mut warnings = Vec::new();
    for entry in fs::read_dir(&quarantine)? {
        manager.check_cancelled()?;
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
            warnings.push("quarantine_unknown_entry".to_owned());
            continue;
        }
        let Some(sha256) = entry.file_name().to_str().map(str::to_owned) else {
            warnings.push("quarantine_invalid_name".to_owned());
            continue;
        };
        let expected = match expected_relative_path(&sha256) {
            Ok(value) => value,
            Err(_) => {
                warnings.push("quarantine_invalid_name".to_owned());
                continue;
            }
        };
        let stored = {
            let connection = store.connection()?;
            connection
                .query_row(
                    "select relative_path from content_objects where sha256 = ?1",
                    [&sha256],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        };
        if let Some(stored_relative_path) = stored {
            if Path::new(&stored_relative_path) != expected {
                warnings.push("quarantine_unsafe_database_path".to_owned());
                continue;
            }
            let destination = store.content_directory().join(expected);
            fs::create_dir_all(destination.parent().ok_or(ContentError::UnsafePath)?)?;
            ensure_not_symlink(destination.parent().ok_or(ContentError::UnsafePath)?)?;
            let (quarantine_hash, _) = hash_file(&path, manager)?;
            if quarantine_hash != sha256 {
                warnings.push("quarantine_hash_mismatch".to_owned());
                continue;
            }
            if destination.exists() {
                let (destination_hash, _) = hash_file(&destination, manager)?;
                if destination_hash != sha256 {
                    warnings.push("object_hash_conflict".to_owned());
                    continue;
                }
                fs::remove_file(path)?;
            } else {
                fs::rename(path, destination)?;
            }
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(warnings)
}

fn sweep_candidates(
    store: &AppStore,
    manager: &ContentGcManager,
    include_grace_period: bool,
) -> Result<(usize, u64, Vec<String>), ContentError> {
    sweep_candidates_with_hook(store, manager, include_grace_period, |_, _| Ok(()))
}

fn sweep_candidates_with_hook<F>(
    store: &AppStore,
    manager: &ContentGcManager,
    include_grace_period: bool,
    mut after_quarantine: F,
) -> Result<(usize, u64, Vec<String>), ContentError>
where
    F: FnMut(&AppStore, &str) -> Result<(), ContentError>,
{
    {
        let connection = store.connection()?;
        ensure_backfill_complete(&connection)?;
    }
    let candidates = {
        let connection = store.connection()?;
        let mut statement = connection.prepare(
            "select g.content_sha256, c.relative_path, c.byte_size
             from content_object_gc_candidates g
             join content_objects c on c.sha256 = g.content_sha256
             where (?1 = 1 or julianday(g.not_before) <= julianday('now'))
             order by g.not_before, g.content_sha256",
        )?;
        let rows = statement.query_map([include_grace_period], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut candidates = Vec::with_capacity(SWEEP_OBJECT_LIMIT);
        for row in rows {
            manager.check_cancelled()?;
            let candidate = row?;
            // Session roots only grow. Skip them before the batch limit so a
            // long-lived editor cannot starve unrelated orphan reclamation.
            if store.is_content_session_retained(&candidate.0)? {
                continue;
            }
            candidates.push(candidate);
            if candidates.len() == SWEEP_OBJECT_LIMIT {
                break;
            }
        }
        candidates
    };
    let quarantine = store.content_directory().join(".gc");
    fs::create_dir_all(&quarantine)?;
    ensure_not_symlink(&quarantine)?;
    let mut deleted_count = 0;
    let mut deleted_bytes = 0_u64;
    let mut warnings = Vec::new();
    for (sha256, relative_path, stored_byte_size) in candidates {
        manager.check_cancelled()?;
        if deleted_bytes > 0
            && deleted_bytes.saturating_add(stored_byte_size.max(0) as u64) > SWEEP_BYTE_LIMIT
        {
            break;
        }
        let source = match validate_object_path(store.content_directory(), &sha256, &relative_path)
        {
            Ok(value) => value,
            Err(_) => {
                let connection = store.connection()?;
                record_candidate_error(&connection, &sha256, "unsafe_path")?;
                warnings.push("unsafe_path".to_owned());
                continue;
            }
        };
        let quarantined = quarantine.join(&sha256);
        ensure_not_symlink(&quarantined)?;
        let verified_size = if source.exists() {
            let (actual_hash, byte_size) = hash_file(&source, manager)?;
            if actual_hash != sha256 {
                let connection = store.connection()?;
                record_candidate_error(&connection, &sha256, "hash_mismatch")?;
                warnings.push("hash_mismatch".to_owned());
                continue;
            }
            Some(byte_size)
        } else {
            None
        };

        // File hashing is intentionally outside both the SQLite mutex and the
        // content maintenance lock. The write lock covers only the final
        // reference check, atomic move, and metadata commit.
        let _content_write = store.content_write()?;
        // Native undo is process-owned, beyond the latest durable S/B/S-prime.
        // A forced sweep may bypass time, but never a live process owner.
        if store.is_content_session_retained(&sha256)? {
            continue;
        }
        {
            let connection = store.connection()?;
            if has_references(&connection, &sha256)? {
                connection.execute(
                    "delete from content_object_gc_candidates where content_sha256 = ?1",
                    [&sha256],
                )?;
                continue;
            }
        }
        let actual_size = if source.exists() {
            if let Err(error) = fs::rename(&source, &quarantined) {
                let connection = store.connection()?;
                record_candidate_error(&connection, &sha256, "quarantine_move_failed")?;
                warnings.push(format!("quarantine_move_failed:{}", error.kind()));
                continue;
            }
            after_quarantine(store, &sha256)?;
            verified_size.unwrap_or(0)
        } else {
            0
        };
        let delete_result = (|| -> Result<bool, ContentError> {
            let mut connection = store.connection()?;
            let transaction = connection.transaction()?;
            let removed = transaction.execute(
                "delete from content_objects where sha256 = ?1
                 and not exists(select 1 from content_object_references where content_sha256 = ?1)",
                [&sha256],
            )?;
            if removed == 0 {
                transaction.rollback()?;
                connection.execute(
                    "delete from content_object_gc_candidates where content_sha256 = ?1",
                    [&sha256],
                )?;
                return Ok(false);
            }
            transaction.commit()?;
            Ok(true)
        })();
        match delete_result {
            Ok(true) => {}
            Ok(false) => {
                if quarantined.exists() {
                    fs::create_dir_all(source.parent().ok_or(ContentError::UnsafePath)?)?;
                    fs::rename(&quarantined, &source)?;
                }
                continue;
            }
            Err(error) => {
                if quarantined.exists() {
                    fs::create_dir_all(source.parent().ok_or(ContentError::UnsafePath)?)?;
                    fs::rename(&quarantined, &source)?;
                }
                return Err(error);
            }
        }
        if quarantined.exists()
            && let Err(error) = fs::remove_file(&quarantined)
        {
            warnings.push(format!("quarantine_cleanup_failed:{}", error.kind()));
        }
        deleted_count += 1;
        deleted_bytes =
            deleted_bytes.saturating_add(actual_size.max(stored_byte_size.max(0) as u64));
    }
    let connection = store.connection()?;
    connection.execute(
        "update content_gc_state set last_sweep_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             last_success_count = ?1, last_success_bytes = ?2, job_state = 'idle',
             last_error_code = null, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = 1",
        params![deleted_count as i64, deleted_bytes as i64],
    )?;
    Ok((deleted_count, deleted_bytes, warnings))
}

pub(crate) fn run_gc(
    store: &AppStore,
    manager: &ContentGcManager,
    include_grace_period: bool,
) -> Result<ContentGcRunReport, ContentError> {
    set_persistent_job_state(store, "running", None)?;
    let result = run_gc_inner(store, manager, include_grace_period);
    if let Err(error) = &result {
        let state = if matches!(error, ContentError::Cancelled) {
            "idle"
        } else {
            "failed"
        };
        let _ = set_persistent_job_state(store, state, Some(error.diagnostic_code()));
    }
    result
}

fn run_gc_inner(
    store: &AppStore,
    manager: &ContentGcManager,
    include_grace_period: bool,
) -> Result<ContentGcRunReport, ContentError> {
    manager.set_phase("recovery")?;
    let mut warnings = recover_quarantine(store, manager)?;
    complete_revision_asset_backfill(store, manager)?;
    manager.set_phase("prune")?;
    let pruned_revision_count = {
        let mut connection = store.connection()?;
        revision_assets::prune_revisions(&mut connection)
            .map_err(|error| ContentError::BackfillBlocked(error.to_string()))?
    };
    manager.set_phase("mark")?;
    let marked_object_count = mark_all_candidates(store, manager)?;
    manager.set_phase("sweep")?;
    let (deleted_object_count, deleted_bytes, sweep_warnings) =
        sweep_candidates(store, manager, include_grace_period)?;
    warnings.extend(sweep_warnings);
    Ok(ContentGcRunReport {
        pruned_revision_count,
        marked_object_count,
        deleted_object_count,
        deleted_bytes,
        cleanup_warnings: warnings,
    })
}

fn complete_revision_asset_backfill(
    store: &AppStore,
    manager: &ContentGcManager,
) -> Result<(), ContentError> {
    manager.set_phase("backfill")?;
    loop {
        manager.check_cancelled()?;
        let progress = {
            let mut connection = store.connection()?;
            revision_assets::backfill_revision_assets_batch(&mut connection)
                .map_err(|error| ContentError::BackfillBlocked(error.to_string()))?
        };
        match progress {
            revision_assets::RevisionAssetBackfillProgress::Complete => return Ok(()),
            revision_assets::RevisionAssetBackfillProgress::Pending { .. } => {
                std::thread::yield_now();
            }
            revision_assets::RevisionAssetBackfillProgress::Blocked { error_code } => {
                return Err(ContentError::BackfillBlocked(error_code));
            }
        }
    }
}

fn mark_all_candidates(
    store: &AppStore,
    manager: &ContentGcManager,
) -> Result<usize, ContentError> {
    let mut total = 0;
    loop {
        manager.check_cancelled()?;
        let inserted = {
            let mut connection = store.connection()?;
            reconcile_candidates(&mut connection)?
        };
        total += inserted;
        if inserted < MARK_BATCH_SIZE {
            return Ok(total);
        }
        std::thread::yield_now();
    }
}

pub(crate) fn run_scan(
    store: &AppStore,
    manager: &ContentGcManager,
) -> Result<usize, ContentError> {
    set_persistent_job_state(store, "running", None)?;
    let result = (|| {
        complete_revision_asset_backfill(store, manager)?;
        manager.set_phase("mark")?;
        mark_all_candidates(store, manager)
    })();
    match &result {
        Ok(_) => {
            set_persistent_job_state(store, "idle", None)?;
        }
        Err(error) => {
            let state = if matches!(error, ContentError::Cancelled) {
                "idle"
            } else {
                "failed"
            };
            let _ = set_persistent_job_state(store, state, Some(error.diagnostic_code()));
        }
    }
    result
}

fn set_persistent_job_state(
    store: &AppStore,
    state: &str,
    error_code: Option<&str>,
) -> Result<(), ContentError> {
    let connection = store.connection()?;
    connection.execute(
        "update content_gc_state set job_state = ?1, last_error_code = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = 1",
        params![state, error_code],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "gc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_retention_tests.rs"]
mod session_retention_tests;
