//! Persist the complete original child cleanup before its archive side effect.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, Transaction, diagnostic, receipts,
};
use research_agent_core::conversation::ArchiveConversation;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.child.cleanup.prepare";

#[derive(Clone, Debug)]
pub(crate) struct ChildCleanupIntent {
    pub request_id: Uuid,
    pub archive: ArchiveConversation,
}

#[derive(Clone, Debug)]
pub(crate) struct ChildCleanupPrepared {
    pub intent: ChildCleanupIntent,
    pub epoch: Uuid,
}

impl ChildCleanupIntent {
    pub(crate) fn preparation_id(child: Uuid, request: Uuid) -> Uuid {
        let mut phase = b"agent.child.cleanup.prepare:v1:".to_vec();
        phase.extend_from_slice(request.as_bytes());
        Uuid::new_v5(&child, &phase)
    }

    fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        self.archive
            .validate()
            .map_err(SqlImmediateFailure::Domain)?;
        let child = *self.archive.id.as_uuid();
        let mut archive_phase = b"agent.child.cleanup.archive:v1:".to_vec();
        archive_phase.extend_from_slice(self.request_id.as_bytes());
        if self.request_id.is_nil()
            || !self.archive.archived
            || self.archive.expected_watermark.is_none()
            || self.archive.operation_id != Uuid::new_v5(&child, &archive_phase)
        {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    self.archive.operation_id,
                    None,
                    "child-cleanup-prepare",
                    "invalid_cleanup_intent",
                ),
            )));
        }
        let operation_id = Self::preparation_id(child, self.request_id);
        #[derive(Serialize)]
        struct Frozen {
            kind: &'static str,
            version: u8,
            child_id: Uuid,
            request_id: Uuid,
            archive_operation_id: Uuid,
            expected_watermark: u64,
        }
        let bytes = serde_json::to_vec(&Frozen {
            kind: KIND,
            version: 1,
            child_id: child,
            request_id: self.request_id,
            archive_operation_id: self.archive.operation_id,
            expected_watermark: self.archive.expected_watermark.unwrap_or_default(),
        })
        .map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                operation_id,
                None,
                "child-cleanup-prepare",
                "cleanup_intent_encoding_failed",
            )))
        })?;
        Ok(ReceiptIdentity {
            operation_id,
            kind: KIND,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

impl Database {
    pub(crate) fn prepare_child_cleanup(
        &self,
        intent: &ChildCleanupIntent,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.prepare_child_cleanup_owned(
            intent,
            false,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    pub(crate) fn prepare_unsubmitted_child_cleanup(
        &self,
        intent: &ChildCleanupIntent,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.prepare_child_cleanup_owned(
            intent,
            true,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn prepare_child_cleanup_owned(
        &self,
        intent: &ChildCleanupIntent,
        require_absent: bool,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = intent.identity()?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| register(tx, fence, intent, &identity, require_absent),
            #[cfg(test)]
            fault,
        )
    }

    pub(crate) fn inspect_child_cleanup(
        &self,
        child: Uuid,
    ) -> Result<Option<ChildCleanupPrepared>, SqlImmediateFailure> {
        if child.is_nil() {
            return Err(SqlImmediateFailure::Domain(
                research_agent_core::conversation::CatalogError::InvalidInput,
            ));
        }
        let owner = self.owner.as_ref().ok_or_else(|| super::unmanaged(child))?;
        let inspection = owner
            .inspection_guard()
            .map_err(|error| super::admission_error(child, error, None))?;
        let snapshot = inspection.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                child,
                fence,
                "child-cleanup-status",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() || snapshot.blocked {
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                child,
                fence,
                "child-cleanup-status",
                "cleanup_owner_unconfirmed",
            ))));
        }
        let fence = receipts::status_fence(&db, child, fence, snapshot.accepted_position)?;
        let saved: Option<(String, String, String, u64, String, u32)> = db.query_row(
            "select request_id,prepare_operation_id,archive_operation_id,expected_watermark,epoch,version from agent_child_cleanup_intents where child_id=?1",
            [child.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        ).optional().map_err(|error| receipts::storage(child, fence, "child-cleanup-status", &error))?;
        let Some((request, prepare, archive, watermark, epoch, version)) = saved else {
            return Ok(None);
        };
        let corrupt = || {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                child,
                fence,
                "child-cleanup-status",
                "cleanup_intent_invalid",
            )))
        };
        let request_id = Uuid::parse_str(&request).map_err(|_| corrupt())?;
        let operation_id = Uuid::parse_str(&archive).map_err(|_| corrupt())?;
        if version != 1 || epoch != fence.epoch.to_string() {
            return Err(corrupt());
        }
        let intent = ChildCleanupIntent {
            request_id,
            archive: ArchiveConversation {
                operation_id,
                id: research_agent_core::conversation::ConversationId::from_uuid(child),
                archived: true,
                expected_watermark: Some(watermark),
            },
        };
        let identity = intent.identity().map_err(|_| corrupt())?;
        receipts::check_cross_kind(&db, identity.operation_id, fence, "child-cleanup-status")?;
        if identity.operation_id.to_string() != prepare
            || receipts::lookup(&db, &identity, fence, "child-cleanup-status")?.is_none()
        {
            return Err(corrupt());
        }
        Ok(Some(ChildCleanupPrepared {
            intent,
            epoch: fence.epoch,
        }))
    }
}

fn register(
    tx: &Transaction<'_>,
    fence: ReadFence,
    intent: &ChildCleanupIntent,
    identity: &ReceiptIdentity,
    require_absent: bool,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = identity.operation_id;
    receipts::check_state(tx, operation, fence, "child-cleanup-prepare")?;
    receipts::check_cross_kind(tx, operation, fence, "child-cleanup-prepare")?;
    let child = intent.archive.id.as_uuid().to_string();
    if require_absent {
        let submitted: bool = tx
            .query_row(
                "select exists(select 1 from agent_submissions where conversation_id=?1)",
                [&child],
                |row| row.get(0),
            )
            .map_err(|error| {
                receipts::storage(operation, fence, "child-cleanup-submission-check", &error)
            })?;
        if submitted {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                operation,
                fence,
                "child-cleanup-submission-check",
                "child_submission_already_exists",
            ))));
        }
    }
    let saved: Option<(String, String, String, u64, String, u32)> = tx.query_row(
        "select request_id,prepare_operation_id,archive_operation_id,expected_watermark,epoch,version from agent_child_cleanup_intents where child_id=?1",
        [&child], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).optional().map_err(|error| receipts::storage(operation, fence, "child-cleanup-prepare-read", &error))?;
    let expected = (
        intent.request_id.to_string(),
        operation.to_string(),
        intent.archive.operation_id.to_string(),
        intent.archive.expected_watermark.unwrap_or_default(),
        fence.epoch.to_string(),
        1,
    );
    let receipt = receipts::lookup(tx, identity, fence, "child-cleanup-prepare")?;
    match (saved, receipt) {
        (Some(saved), Some(receipt)) if saved == expected => return Ok(receipt),
        (None, None) => {}
        (Some(saved), _) if saved != expected => {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                operation,
                fence,
                "child-cleanup-prepare",
                "cleanup_intent_changed",
            ))));
        }
        _ => {
            return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "child-cleanup-prepare",
                "cleanup_intent_receipt_mismatch",
            ))));
        }
    }
    tx.execute("insert into agent_child_cleanup_intents(child_id,request_id,prepare_operation_id,archive_operation_id,expected_watermark,epoch,version) values(?1,?2,?3,?4,?5,?6,1)",
        params![child, expected.0, expected.1, expected.2, expected.3, expected.4])
        .map_err(|error| receipts::storage(operation, fence, "child-cleanup-prepare-insert", &error))?;
    receipts::insert(tx, identity, fence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::conversation::ConversationId;

    fn intent(child: Uuid, request: Uuid, watermark: u64) -> ChildCleanupIntent {
        let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
        phase.extend_from_slice(request.as_bytes());
        ChildCleanupIntent {
            request_id: request,
            archive: ArchiveConversation {
                operation_id: Uuid::new_v5(&child, &phase),
                id: ConversationId::from_uuid(child),
                archived: true,
                expected_watermark: Some(watermark),
            },
        }
    }

    #[test]
    fn prepared_intent_is_atomic_idempotent_and_bound_to_epoch() {
        let root = tempfile::tempdir().unwrap();
        let mut sql = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let child = Uuid::new_v4();
        let request = Uuid::new_v4();
        let original = intent(child, request, 0);
        let receipt = db.prepare_child_cleanup(&original).unwrap();
        assert!(!receipt.replayed);
        let saved = db.inspect_child_cleanup(child).unwrap().unwrap();
        assert_eq!(saved.intent.request_id, request);
        assert_eq!(saved.intent.archive, original.archive);
        assert_eq!(saved.epoch, receipt.epoch);
        assert!(db.prepare_child_cleanup(&original).unwrap().replayed);
        let later = intent(Uuid::new_v4(), Uuid::new_v4(), 7);
        db.prepare_child_cleanup(&later).unwrap();
        assert_eq!(
            db.inspect_child_cleanup(*later.archive.id.as_uuid())
                .unwrap()
                .unwrap()
                .intent
                .archive
                .expected_watermark,
            Some(7)
        );
        assert!(matches!(
            db.prepare_child_cleanup(&intent(child, request, 1)),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            db.prepare_child_cleanup(&intent(child, Uuid::new_v4(), 0)),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert_eq!(
            db.inspect_child_cleanup(child)
                .unwrap()
                .unwrap()
                .intent
                .archive,
            original.archive
        );
        db.lock()
            .unwrap()
            .execute(
                "update agent_child_cleanup_intents set expected_watermark=1 where child_id=?1",
                [child.to_string()],
            )
            .unwrap();
        assert!(
            db.inspect_child_cleanup(child).is_err(),
            "a changed intent must not become original proof"
        );
    }

    #[test]
    fn receipt_insert_failure_rolls_back_the_intent() {
        let root = tempfile::tempdir().unwrap();
        let mut sql = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        db.lock().unwrap().execute_batch("create trigger reject_cleanup_prepare_receipt before insert on data_sql_operation_receipts when new.kind='agent.child.cleanup.prepare' begin select raise(abort,'synthetic failure'); end;").unwrap();
        let child = Uuid::new_v4();
        assert!(
            db.prepare_child_cleanup(&intent(child, Uuid::new_v4(), 0))
                .is_err()
        );
        assert!(db.inspect_child_cleanup(child).unwrap().is_none());
    }

    #[test]
    fn submission_snapshot_is_scoped_to_original_child_and_refuses_blocked_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut sql = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let parent = Uuid::new_v4();
        let child = Uuid::new_v4();
        let request = Uuid::new_v4();
        let turn = Uuid::new_v4();
        let context = Uuid::new_v4();
        for id in [parent, child] {
            sql.execute(
                "insert into agent_conversations(id,title) values(?1,'fixture')",
                [id.to_string()],
            )
            .unwrap();
            sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','fixture')", [id.to_string()]).unwrap();
        }
        let binding = research_agent_core::conversation::ConversationBinding {
            kind: research_agent_core::conversation::BindingKind::Paper,
            resource_id: "fixture".into(),
        };
        sql.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')", params![child.to_string(), request.to_string(), turn.to_string(), context.to_string()]).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        assert_eq!(
            db.inspect_child_submission(parent, child, request, &binding)
                .unwrap()
                .record,
            Some((turn, context))
        );
        assert!(
            db.inspect_child_submission(parent, child, Uuid::new_v4(), &binding)
                .unwrap()
                .record
                .is_none()
        );
        assert!(matches!(
            db.inspect_child_submission(parent, Uuid::new_v4(), request, &binding),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        db.lock()
            .unwrap()
            .execute(
                "delete from agent_conversation_bindings where conversation_id=?1",
                [child.to_string()],
            )
            .unwrap();
        assert!(matches!(
            db.inspect_child_submission(parent, child, request, &binding),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        let original = intent(child, Uuid::new_v4(), 0);
        assert!(matches!(
            db.prepare_child_cleanup_owned(&original, false, super::super::Fault::AfterCommit),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        assert!(matches!(
            db.inspect_child_submission(parent, child, request, &binding),
            Err(SqlImmediateFailure::RecoveryRequired(_))
        ));
    }

    #[test]
    fn unknown_preparation_never_claims_absence_until_same_data_reopens() {
        for (fault, committed) in [
            (super::super::Fault::BeforeCommit, false),
            (super::super::Fault::Rollback, false),
            (super::super::Fault::AfterCommit, true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("cleanup.sqlite3");
            let log = root.path().join("data-log");
            let mut sql = rusqlite::Connection::open(&path).unwrap();
            crate::storage::migrate(&mut sql).unwrap();
            let db = Database::open(sql, &log).unwrap();
            let child = Uuid::new_v4();
            let original = intent(child, Uuid::new_v4(), 7);
            assert!(matches!(
                db.prepare_child_cleanup_owned(&original, false, fault),
                Err(SqlImmediateFailure::Unknown(_))
            ));
            assert!(matches!(
                db.inspect_child_cleanup(child),
                Err(SqlImmediateFailure::RecoveryRequired(_))
            ));
            assert!(matches!(
                db.prepare_child_cleanup(&original),
                Err(SqlImmediateFailure::RecoveryRequired(_))
            ));
            drop(db);
            let reopened =
                Database::open(rusqlite::Connection::open(&path).unwrap(), &log).unwrap();
            let saved = reopened.inspect_child_cleanup(child).unwrap();
            assert_eq!(saved.is_some(), committed);
            if let Some(saved) = saved {
                assert_eq!(saved.intent.request_id, original.request_id);
                assert_eq!(saved.intent.archive, original.archive);
            }
            let sql = reopened.lock().unwrap();
            let archives: u64 = sql.query_row(
                "select count(*) from data_sql_operation_receipts where kind='agent.conversation.archive'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(archives, 0);
        }
    }
}
