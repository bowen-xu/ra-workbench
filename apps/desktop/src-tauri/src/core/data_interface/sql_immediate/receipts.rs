//! SQL receipt validation remains inside the caller's admission/SQL lock order.
use super::{
    AuthorizeRollback, ReadFence, SqlAuthorizationError, SqlImmediateFailure, SqlImmediateReceipt,
    Transaction, diagnostic,
    types::ReceiptIdentity,
    types::{KIND, VERSION},
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub(super) fn check_state(
    db: &Connection,
    operation_id: Uuid,
    fence: ReadFence,
    phase: &'static str,
) -> Result<(), SqlImmediateFailure> {
    let state: Option<(String, u64, String)> = db.query_row(
        "select epoch,materialized_position,bootstrap_state from data_runtime_state where singleton=1 and schema_version=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(|error| storage(operation_id, fence, phase, &error))?;
    if state != Some((fence.epoch.to_string(), fence.position, "ready".into())) {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "sql_pair_or_fence_mismatch",
        ))));
    }
    Ok(())
}

pub(super) fn status_fence(
    db: &Connection,
    operation_id: Uuid,
    cached: ReadFence,
    accepted_position: u64,
) -> Result<ReadFence, SqlImmediateFailure> {
    let position: Option<u64> = db
        .query_row(
            "select materialized_position from data_runtime_state
         where singleton=1 and schema_version=1 and epoch=?1 and bootstrap_state='ready'",
            [cached.epoch.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage(operation_id, cached, "status", &error))?;
    let Some(position) =
        position.filter(|position| *position >= cached.position && *position <= accepted_position)
    else {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation_id,
            cached,
            "status",
            "sql_pair_or_fence_mismatch",
        ))));
    };
    Ok(ReadFence {
        epoch: cached.epoch,
        position,
    })
}

pub(super) fn check_cross_kind(
    db: &Connection,
    operation_id: Uuid,
    fence: ReadFence,
    phase: &'static str,
) -> Result<(), SqlImmediateFailure> {
    let collision: bool = db
        .query_row(
            "select exists(select 1 from data_operation_receipts where operation_id=?1)
         or exists(select 1 from data_journal_operation_ids where operation_id=?1)",
            [operation_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|error| storage(operation_id, fence, phase, &error))?;
    if collision {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "operation_id_already_belongs_to_journal",
        ))));
    }
    Ok(())
}

pub(super) fn lookup(
    db: &Connection,
    identity: &ReceiptIdentity,
    fence: ReadFence,
    phase: &'static str,
) -> Result<Option<SqlImmediateReceipt>, SqlImmediateFailure> {
    let result = lookup_with_result(db, identity, fence, phase, true)?;
    Ok(result.map(|(receipt, _)| receipt))
}

pub(super) fn lookup_with_result(
    db: &Connection,
    identity: &ReceiptIdentity,
    fence: ReadFence,
    phase: &'static str,
    require_unit: bool,
) -> Result<Option<(SqlImmediateReceipt, String)>, SqlImmediateFailure> {
    let operation_id = identity.operation_id;
    let fingerprint = identity.fingerprint;
    let row: Option<(String, String, u32, Vec<u8>, u64, String)> = db
        .query_row(
            "select epoch,kind,version,fingerprint,sql_fence_position,result_json
         from data_sql_operation_receipts where operation_id=?1",
            [operation_id.to_string()],
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
        )
        .optional()
        .map_err(|error| storage(operation_id, fence, phase, &error))?;
    let Some((epoch, kind, version, saved_hash, position, result)) = row else {
        return Ok(None);
    };
    if epoch != fence.epoch.to_string() || position > fence.position || saved_hash.len() != 32 {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "sql_receipt_invalid",
        ))));
    }
    if kind != identity.kind || version != identity.version {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "operation_id_content_changed",
        ))));
    }
    if require_unit && serde_json::from_str::<()>(&result).is_err() {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "sql_receipt_result_invalid",
        ))));
    }
    if saved_hash != fingerprint {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation_id,
            fence,
            phase,
            "operation_id_content_changed",
        ))));
    }
    Ok(Some((
        SqlImmediateReceipt {
            operation_id,
            epoch: fence.epoch,
            sql_fence: ReadFence {
                epoch: fence.epoch,
                position,
            },
            fingerprint,
            result: (),
            replayed: true,
        },
        result,
    )))
}

pub(super) fn register(
    tx: &Transaction<'_>,
    command: &AuthorizeRollback,
    fence: ReadFence,
    fingerprint: [u8; 32],
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation_id = command.operation_id;
    let identity = ReceiptIdentity {
        operation_id,
        kind: KIND,
        version: VERSION,
        fingerprint,
    };
    check_state(tx, operation_id, fence, "prepare")?;
    // Keep the old liveness check before any replay shortcut. An old receipt
    // cannot authorize a deleted resource or a revoked binding.
    crate::agent::repository::authorize_rollback_in(tx, command).map_err(|error| match error {
        SqlAuthorizationError::Domain(error) => SqlImmediateFailure::Domain(error),
        SqlAuthorizationError::Sql(error) => storage(operation_id, fence, "authorize", &error),
    })?;
    check_cross_kind(tx, operation_id, fence, "prepare")?;
    if let Some(receipt) = lookup(tx, &identity, fence, "prepare")? {
        return Ok(receipt);
    }
    insert(tx, &identity, fence)
}

pub(super) fn insert(
    tx: &Transaction<'_>,
    identity: &ReceiptIdentity,
    fence: ReadFence,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation_id = identity.operation_id;
    tx.execute(
        "insert into data_sql_operation_receipts
         (operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json)
         values(?1,?2,?3,?4,?5,?6,'null')",
        params![
            identity.operation_id.to_string(),
            fence.epoch.to_string(),
            identity.kind,
            identity.version,
            identity.fingerprint.as_slice(),
            fence.position
        ],
    )
    .map_err(|error| storage(operation_id, fence, "receipt", &error))?;
    Ok(SqlImmediateReceipt {
        operation_id: identity.operation_id,
        epoch: fence.epoch,
        sql_fence: fence,
        fingerprint: identity.fingerprint,
        result: (),
        replayed: false,
    })
}

pub(super) fn storage(
    operation_id: Uuid,
    fence: ReadFence,
    phase: &'static str,
    error: &rusqlite::Error,
) -> SqlImmediateFailure {
    SqlImmediateFailure::Unavailable(Box::new(
        diagnostic(operation_id, fence, phase, "sql_registration_failed").sqlite(error),
    ))
}
