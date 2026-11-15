//! Write-ahead discovery for interrupted object writes, independent of sessions.
use super::{LeaseError, ProposalObjects};
use crate::storage::AppStore;
use research_agent_core::workspace::FileVersion;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

#[cfg(test)]
#[path = "object_staging/tests.rs"]
mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Lease(#[from] LeaseError),
    #[error("object staging storage unavailable")]
    Storage,
    #[error("object staging requires recovery")]
    RecoveryRequired,
}

fn registered(store: &AppStore, version: &FileVersion) -> Result<Option<String>, Error> {
    let db = store.connection().map_err(|_| Error::Storage)?;
    let row: Option<(String, u64)> = db
        .query_row(
            "select relative_path,byte_size from content_objects where sha256=?1",
            [&version.sha256],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| Error::Storage)?;
    match row {
        Some((path, size)) if size == version.byte_size => Ok(Some(path)),
        Some(_) => Err(LeaseError::Conflict.into()),
        None => Ok(None),
    }
}

pub(crate) fn stage(
    store: &AppStore,
    objects: &ProposalObjects,
    version: &FileVersion,
    bytes: &[u8],
) -> Result<(), Error> {
    if let Some(path) = registered(store, version)? {
        objects.verify_registered(version, &path)?;
        return Ok(());
    }
    // This autocommit MUST finish before any new file is created.
    {
        let db = store.connection().map_err(|_| Error::Storage)?;
        db.execute("insert into agent_object_staging(sha256,byte_size) values(?1,?2) on conflict(sha256) do nothing", params![version.sha256,version.byte_size]).map_err(|_| Error::Storage)?;
        let saved: u64 = db
            .query_row(
                "select byte_size from agent_object_staging where sha256=?1",
                [&version.sha256],
                |r| r.get(0),
            )
            .map_err(|_| Error::Storage)?;
        if saved != version.byte_size {
            return Err(LeaseError::Conflict.into());
        }
    }
    let path = objects.stage(version, bytes)?;
    register(store, objects, version, &path)
}

fn register(
    store: &AppStore,
    objects: &ProposalObjects,
    version: &FileVersion,
    path: &str,
) -> Result<(), Error> {
    // Callers hold content_write before acquiring the DB mutex.
    objects.verify_registered(version, path)?;
    let mut db = store.connection().map_err(|_| Error::Storage)?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
    let prepared = (|| -> Result<(), Error> {
        tx.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'application/octet-stream',?3,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(sha256) do nothing", params![version.sha256,path,version.byte_size]).map_err(|_| Error::Storage)?;
        let (saved_path, saved_size): (String, u64) = tx
            .query_row(
                "select relative_path,byte_size from content_objects where sha256=?1",
                [&version.sha256],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::Storage)?;
        if saved_size != version.byte_size {
            return Err(LeaseError::Conflict.into());
        }
        objects.verify_registered(version, &saved_path)?;
        tx.execute("insert into content_object_gc_candidates(content_sha256,unreferenced_since,not_before,reason) select ?1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 days'),'live_reference_removed' where not exists(select 1 from content_object_references where content_sha256=?1) on conflict(content_sha256) do nothing", [&version.sha256]).map_err(|_| Error::Storage)?;
        tx.execute(
            "delete from agent_object_staging where sha256=?1",
            [&version.sha256],
        )
        .map_err(|_| Error::Storage)?;
        Ok(())
    })();
    match prepared {
        Ok(()) => match tx.commit() {
            Ok(()) if db.is_autocommit() => Ok(()),
            _ => {
                if !db.is_autocommit() {
                    let _ = db.execute_batch("rollback");
                }
                db.require_recovery();
                Err(Error::RecoveryRequired)
            }
        },
        Err(error) => match tx.rollback() {
            Ok(()) if db.is_autocommit() => Err(error),
            _ => {
                db.require_recovery();
                Err(Error::RecoveryRequired)
            }
        },
    }
}

pub(crate) fn recover(store: &AppStore) -> Result<(usize, usize), Error> {
    let _content = store.content_write().map_err(|_| Error::Storage)?;
    let pending = {
        let db = store.connection().map_err(|_| Error::Storage)?;
        let mut statement = db.prepare("select sha256,byte_size from agent_object_staging order by coalesce(last_attempt_at,''),created_at,sha256 limit 100").map_err(|_| Error::Storage)?;
        statement
            .query_map([], |r| {
                Ok(FileVersion {
                    sha256: r.get(0)?,
                    byte_size: r.get(1)?,
                    executable: false,
                })
            })
            .map_err(|_| Error::Storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Storage)?
    };
    let objects = ProposalObjects::open(
        &store
            .content_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?,
    )?;
    let mut recovered = 0;
    let mut unresolved = 0;
    let mut budget = 256 * 1024 * 1024u64;
    for version in pending {
        if version.byte_size > budget {
            break;
        }
        budget -= version.byte_size;
        store.connection().map_err(|_| Error::Storage)?.execute("update agent_object_staging set last_attempt_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where sha256=?1", [&version.sha256]).map_err(|_| Error::Storage)?;
        let path = registered(store, &version)?.unwrap_or_else(|| {
            format!(
                "{}/{}/{}",
                &version.sha256[..2],
                &version.sha256[2..4],
                version.sha256
            )
        });
        match register(store, &objects, &version, &path) {
            Ok(()) => recovered += 1,
            Err(Error::RecoveryRequired) => return Err(Error::RecoveryRequired),
            Err(_) => unresolved += 1,
        }
    }
    Ok((recovered, unresolved))
}
