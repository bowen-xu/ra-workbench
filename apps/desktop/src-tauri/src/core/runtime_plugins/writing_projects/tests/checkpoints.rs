use super::files::{journal, seed, store};
use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{
        FileRequest, FileResult, checkpoints::CheckpointCall, operate_files,
    },
};
use research_workbench_native_sdk::writing_checkpoint::{
    CheckpointRecord, CheckpointWrite, DocumentCheckpoint,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn document(id: &str) -> DocumentCheckpoint {
    let mut checkpoint: DocumentCheckpoint = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/writing/document-checkpoint-editor-receipt-v1.json"
    )))
    .unwrap();
    checkpoint.manuscript_id = id.into();
    checkpoint
}
async fn save(
    fixture: &Fixture,
    store: &AppStore,
    write: CheckpointWrite,
) -> NativeResult<CheckpointRecord> {
    match operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        write.checkpoint.manuscript_id.clone(),
        FileRequest::Checkpoint {
            call: CheckpointCall::Save {
                write: Box::new(write),
            },
        },
    )
    .await?
    {
        FileResult::Checkpoint(record) => Ok(*record),
        _ => panic!("checkpoint result"),
    }
}
async fn list(
    fixture: &Fixture,
    store: &AppStore,
    id: &str,
) -> NativeResult<Vec<CheckpointRecord>> {
    match operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        id.into(),
        FileRequest::Checkpoint {
            call: CheckpointCall::List,
        },
    )
    .await?
    {
        FileResult::Checkpoints(records) => Ok(records),
        _ => panic!("checkpoint list"),
    }
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_preserves_original_receipt_keep_cas_and_reopen() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let checkpoint = document(&fixture.active);
    let first = save(
        &fixture,
        &store,
        CheckpointWrite {
            expected_revision: None,
            checkpoint: checkpoint.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.checkpoint, checkpoint);
    let mut kept = checkpoint.clone();
    kept.version += 1;
    kept.review_baseline = kept.content.clone();
    let write = CheckpointWrite {
        expected_revision: Some(1),
        checkpoint: kept.clone(),
    };
    let second = save(&fixture, &store, write.clone()).await.unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(second.checkpoint.editor_receipt, checkpoint.editor_receipt);
    assert_eq!(save(&fixture, &store, write).await.unwrap(), second);
    let error = save(
        &fixture,
        &store,
        CheckpointWrite {
            expected_revision: Some(2),
            checkpoint,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "checkpoint_conflict");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        list(&fixture, &store, &fixture.active).await.unwrap(),
        vec![second.clone()]
    );
    let reopened =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    let row:(i64,String)=reopened.query_row("select revision,checkpoint_json from writing_document_checkpoints where manuscript_id=?1",[&fixture.active],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(row.0, 2);
    assert_eq!(row.1, serde_json::to_string(&kept).unwrap());
    for path in ["z.tex", "a.tex"] {
        let mut checkpoint = kept.clone();
        checkpoint.relative_path = path.into();
        save(
            &fixture,
            &store,
            CheckpointWrite {
                expected_revision: None,
                checkpoint,
            },
        )
        .await
        .unwrap();
    }
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&reads);
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "writing_document_checkpoints",
                    column_name: "checkpoint_json"
                }
            ) {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Authorization::Allow
        }));
    let paths = list(&fixture, &store, &fixture.active)
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.checkpoint.relative_path)
        .collect::<Vec<_>>();
    let mut expected = vec![kept.relative_path, "z.tex".into(), "a.tex".into()];
    expected.sort();
    assert_eq!(paths, expected);
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        2 * expected.len()
    );
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_keeps_raw_scope_path_validation_and_missing_owner_order() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let mut checkpoint = document("missing");
    checkpoint.relative_path = "../main.tex".into();
    assert_eq!(
        save(
            &fixture,
            &store,
            CheckpointWrite {
                expected_revision: None,
                checkpoint: checkpoint.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        "invalid_input"
    );
    checkpoint.relative_path = "main.tex".into();
    assert_eq!(
        save(
            &fixture,
            &store,
            CheckpointWrite {
                expected_revision: None,
                checkpoint
            }
        )
        .await
        .unwrap_err()
        .code,
        "not_found"
    );
    let raw = format!(" {} ", fixture.active);
    assert_eq!(
        list(&fixture, &store, &raw).await.unwrap_err().code,
        "not_found"
    );
    assert_eq!(
        save(
            &fixture,
            &store,
            CheckpointWrite {
                expected_revision: None,
                checkpoint: document(&raw)
            }
        )
        .await
        .unwrap_err()
        .code,
        "not_found"
    );
    assert!(
        list(&fixture, &store, &fixture.active)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_recovers_original_pending_bibliography_before_rejecting_input() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let before = seed(&store, &fixture.active, "main.tex", "before");
    journal(
        &fixture,
        &[
            research_workbench_native_sdk::writing_project::CitationKeyWrite {
                path: "main.tex".into(),
                previous: before,
                content: "after".into(),
            },
        ],
    );
    let mut checkpoint = document(&fixture.active);
    checkpoint.schema_version = 2;
    let error = save(
        &fixture,
        &store,
        CheckpointWrite {
            expected_revision: None,
            checkpoint,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(
        error.save_outcome,
        Some(SaveOutcome::Committed {
            item_id: fixture.active.clone()
        })
    );
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "after"
    );
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_citation_key_writes",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from writing_document_checkpoints",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_sql_failure_rolls_back_and_original_request_can_retry() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let write = CheckpointWrite {
        expected_revision: None,
        checkpoint: document(&fixture.active),
    };
    fixture.database.lock().unwrap().execute_batch("create temp trigger checkpoint_failure before insert on writing_document_checkpoints begin select raise(fail,'PRIVATE_CHECKPOINT_FAILURE');end;").unwrap();
    let error = save(&fixture, &store, write.clone()).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("PRIVATE_CHECKPOINT_FAILURE")
    );
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    {
        let db = fixture.database.lock().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row(
                "select count(*) from writing_document_checkpoints",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger checkpoint_failure").unwrap();
    }
    assert_eq!(save(&fixture, &store, write).await.unwrap().revision, 1);
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_unknown_read_rollback_fences_before_publication() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = save(
        &fixture,
        &store,
        CheckpointWrite {
            expected_revision: None,
            checkpoint: document(&fixture.active),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.save_rollback_error.unwrap().phase,
        "checkpoint_rollback"
    );
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    assert!(fixture.database.lock().is_err());
    let reopened =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "select count(*) from writing_document_checkpoints",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_list_consumes_earlier_json_error_before_later_sql_failure() {
    use rusqlite::functions::FunctionFlags;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let store = store(&fixture);
    for path in ["a.tex", "z.tex"] {
        let mut checkpoint = document(&fixture.active);
        checkpoint.relative_path = path.into();
        save(
            &fixture,
            &store,
            CheckpointWrite {
                expected_revision: None,
                checkpoint,
            },
        )
        .await
        .unwrap();
    }
    let later = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&later);
    {
        let db = fixture.database.lock().unwrap();
        db.execute("update writing_document_checkpoints set checkpoint_json='{}' where relative_path='a.tex'",[]).unwrap();
        db.execute_batch("alter table writing_document_checkpoints rename to checkpoint_fixture_rows;create view writing_document_checkpoints as select manuscript_id,relative_path,revision,checkpoint_json_gate(relative_path,checkpoint_json) as checkpoint_json,updated_at from checkpoint_fixture_rows;").unwrap();
        db.create_scalar_function(
            "checkpoint_json_gate",
            2,
            FunctionFlags::SQLITE_UTF8,
            move |context| {
                let path: String = context.get(0)?;
                if path == "z.tex" {
                    observed.fetch_add(1, Ordering::SeqCst);
                    return Err(rusqlite::Error::UserFunctionError(Box::new(
                        std::io::Error::other("PRIVATE_LATE_CHECKPOINT_FAILURE"),
                    )));
                }
                context.get::<String>(1)
            },
        )
        .unwrap();
    }
    let error = list(&fixture, &store, &fixture.active).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(error.message.contains("writing checkpoint JSON"));
    assert!(!error.message.contains("PRIVATE_LATE_CHECKPOINT_FAILURE"));
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    assert_eq!(later.load(Ordering::SeqCst), 0);
    fixture.service.shutdown().await.unwrap();
}

mod race;
