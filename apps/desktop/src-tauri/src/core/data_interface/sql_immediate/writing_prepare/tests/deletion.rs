//! A legal catalog deletion retires a result without corrupting the SQL owner.
use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    ArchiveConversation, BindingKind, ConversationBinding, ConversationCatalog,
    ConversationDeletionRequest, ConversationDeletionStore, ConversationId,
    CreateBoundConversation,
};

fn proof(f: &Fixture) -> (Vec<u8>, String, u64, String) {
    f.db.raw.lock().unwrap().query_row("select fingerprint,epoch,sql_fence_position,result_json from data_sql_operation_receipts where operation_id=?1",
        [f.intent.operation().to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
}

#[tokio::test]
async fn legal_catalog_deletion_retains_original_proof_without_blocking_unrelated_preparation() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        f.prepare().unwrap();
        let original = proof(&f);
        super::ordering::accept_pending(&f);
        let catalog = SqliteConversationCatalog::from_app_store(&f.store);
        let c = ConversationId::from_uuid(f.intent.conversation());
        catalog
            .set_archived(ArchiveConversation {
                operation_id: Uuid::new_v4(),
                id: c,
                archived: true,
                expected_watermark: Some(2),
            })
            .await
            .unwrap();
        catalog
            .delete(ConversationDeletionRequest {
                request_id: Uuid::new_v4(),
                conversation_id: c,
                binding: Some(ConversationBinding {
                    kind: BindingKind::Manuscript,
                    resource_id: f.intent.manuscript().to_string(),
                }),
                expected_title: "Synthetic preparation".into(),
                expected_durable_count: 2,
                force: false,
            })
            .await
            .unwrap();
        assert_eq!(
            f.db.raw
                .lock()
                .unwrap()
                .query_row(
                    "select materialized_position from data_runtime_state",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        let path = f
            .store
            .manuscript_directory()
            .join(f.intent.manuscript().to_string())
            .join("source/main.tex");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(f.count("agent_submissions"), 0);
        assert_eq!(f.count("agent_turn_resource_grants"), 0);
        assert_eq!(f.count("agent_turn_shell_grants"), 0);
        assert_eq!(f.count("agent_compaction_preparations"), 0);
        let WritingPrepareStatus::Unconfirmed(detail) =
            f.db.writing_prepare_status(&f.intent).unwrap()
        else {
            panic!("deleted results cannot fabricate a preview")
        };
        assert_eq!(detail.safe_cause, "writing_prepare_result_deleted");
        assert_eq!(detail.operation_id, f.intent.operation());
        assert_eq!(
            detail.epoch.map(|value| value.to_string()),
            Some(original.1.clone())
        );
        assert_eq!(detail.sql_fence, Some("0".into()));
        for result in [
            f.db.replay_writing_prepare(&f.intent).map(|_| ()),
            f.prepare().map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
            ));
        }
        assert_eq!(proof(&f), original);
        assert_eq!(f.trace(), "sent");
        assert_eq!(f.count("agent_items"), 0);
        // A different conversation still crosses the real preparation write boundary.
        let next = Uuid::new_v4();
        catalog
            .create_bound(CreateBoundConversation {
                operation_id: Uuid::new_v4(),
                id: ConversationId::from_uuid(next),
                binding: ConversationBinding {
                    kind: BindingKind::Manuscript,
                    resource_id: f.intent.manuscript().to_string(),
                },
            })
            .await
            .unwrap();
        std::fs::write(path, "Frozen manuscript").unwrap();
        let mut intent = f.intent.clone();
        intent.request.conversation_id = next;
        intent.request.request_id = Uuid::new_v4();
        intent.request.retrieval = None;
        let mut source = f.source();
        source.request = intent.request.clone().into();
        assert_eq!(
            f.db.prepare_writing(&intent, source)
                .unwrap()
                .conversation_id,
            next
        );
        assert_eq!(f.count("agent_submissions"), 1);
        assert_eq!(proof(&f), original);
    }
}

#[test]
fn missing_result_without_original_tombstone_is_corrupt_and_blocks_write_admission() {
    let f = Fixture::new(false);
    f.prepare().unwrap();
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from agent_submissions where request_id=?1",
            [f.intent.operation().to_string()],
        )
        .unwrap();
    let before = f.db.raw.lock().unwrap().total_changes();
    assert!(matches!(
        f.db.writing_prepare_status(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert_eq!(f.db.raw.lock().unwrap().total_changes(), before);
    assert!(matches!(
        f.db.replay_writing_prepare(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.db.lock().is_err());
}

#[test]
fn unavailable_record_read_retains_sql_codes_without_claiming_corruption_or_blocking_owner() {
    let f = Fixture::new(false);
    let original = f.prepare().unwrap();
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch(
            "alter table agent_submissions rename to synthetic_inaccessible_submissions;",
        )
        .unwrap();
    for result in [
        f.db.writing_prepare_status(&f.intent).map(|_| ()),
        f.db.replay_writing_prepare(&f.intent).map(|_| ()),
    ] {
        let error = result
            .map(|_| ())
            .expect_err("actual SQL read failure required");
        assert_eq!(error.code(), "DATA_SQL_UNAVAILABLE");
        let detail = error.diagnostic().unwrap();
        assert_eq!(detail.phase, "writing-prepare-read");
        assert_eq!(detail.operation_id, f.intent.operation());
        assert_eq!(detail.sqlite_code.as_deref(), Some("Unknown"));
        assert_eq!(
            detail.sqlite_extended_code,
            Some(rusqlite::ffi::SQLITE_ERROR)
        );
    }
    assert!(f.db.lock().is_ok());
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch(
            "alter table synthetic_inaccessible_submissions rename to agent_submissions;",
        )
        .unwrap();
    assert_eq!(
        f.db.replay_writing_prepare(&f.intent).unwrap(),
        Some(original)
    );
}
