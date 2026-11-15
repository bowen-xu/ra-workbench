use super::*;
use crate::agent::{
    repository::{SqliteConversationStore, SqliteSubmissionStore},
    tasks::TaskCompletion,
};
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
use rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn pending_legacy_guard_blocks_a_new_turn_even_when_the_empty_writer_is_live() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("submission.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = crate::storage::AppStore::for_test_database(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    service.create(id).await.unwrap();
    let (_, _, _, example) = setup("guarded input").await;
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let record = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), example.input)
        .await
        .unwrap();
    store
        .connection()
        .unwrap()
        .execute(
            "insert into agent_legacy_import_recovery_guards(conversation_id) values(?1)",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let worker_called = called.clone();
    let result = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |_, _, _| async move {
                worker_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(
        matches!(result, Err(SubmissionRunError::Session(AgentServiceError::LegacyMigrationIncomplete(found))) if found == *id.as_uuid())
    );
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert!(
        service
            .tasks()
            .find_turn_receipt(id, record.turn_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service.read_since(id, 0, 10).await.unwrap().durable_count,
        0
    );
    store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_legacy_import_recovery_guards where conversation_id=?1",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    let history = service.open(id).await.unwrap();
    assert_eq!(history.seq(), 0);
    drop(submissions);
    drop(service);
    store.shared_agent_connection().shutdown().unwrap();
}

async fn setup(
    text: &str,
) -> (
    Arc<Mutex<Connection>>,
    Arc<AgentSessionService>,
    SqliteSubmissionStore,
    SubmissionRecord,
) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 69);
    let db = Arc::new(Mutex::new(db));
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "test".into()).unwrap(),
    )));
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = SqliteSubmissionStore::new(db.clone());
    let record = submissions
        .put(
            id,
            Uuid::new_v4(),
            SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                text: text.into(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "test".into(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
            },
        )
        .await
        .unwrap();
    (db, service, submissions, record)
}
async fn finish(service: &AgentSessionService, record: &SubmissionRecord) {
    service
        .commit_payload(
            ConversationId::from_uuid(record.conversation_id),
            record.request_id,
            EventPayload::TurnFinished {
                turn_id: record.turn_id,
                outcome: TurnOutcome::Completed,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn completed_result_with_unconfirmed_receipt_blocks_submission_after_reopen() {
    let (db, service, submissions, record) = setup("receipt recovery").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let call_id = Uuid::new_v4();
    let result_id = Uuid::new_v4();
    let worker = service.clone();
    let admission = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                for payload in [
                    EventPayload::ItemAccepted {
                        turn_id: record.turn_id,
                        item_id: Uuid::new_v4(),
                        kind: ItemKind::ToolCall {
                            call_id,
                            name: "effect".into(),
                            provider_call_id: None,
                            namespace: None,
                        },
                        text: "{}".into(),
                        phase: None,
                        source: None,
                    },
                    EventPayload::ToolReceiptPending {
                        turn_id: record.turn_id,
                        call_id,
                    },
                    EventPayload::ItemStarted {
                        turn_id: record.turn_id,
                        item_id: result_id,
                        kind: ItemKind::ToolResult {
                            call_id,
                            is_error: Some(false),
                        },
                    },
                    EventPayload::ItemCompleted {
                        turn_id: record.turn_id,
                        item_id: result_id,
                    },
                ] {
                    worker
                        .commit_payload(id, record.request_id, payload)
                        .await
                        .map_err(|_| RunError::EventSink)?;
                }
                Err(RunError::EventSink)
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("initial admission")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let before = service.open(id).await.unwrap();
    assert!(before.has_unknown_tool_outcome());
    assert_eq!(
        before.turn(record.turn_id).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    service.shutdown().await.unwrap();
    let reopened = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db, "test".into()).unwrap(),
    )));
    assert!(reopened.open(id).await.unwrap().has_unknown_tool_outcome());
    let next = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), record.input)
        .await
        .unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let worker_called = called.clone();
    let blocked = reopened
        .start_submission(
            &submissions,
            id,
            next.request_id,
            move |_, _, _| async move {
                worker_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(matches!(
        blocked,
        Err(SubmissionRunError::Session(AgentServiceError::Session(
            SessionError::RecoveryRequired
        )))
    ));
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert!(matches!(
        reopened
            .rollback_conversation(super::super::ConversationRollbackRequest {
                conversation_id: id,
                request_id: Uuid::new_v4(),
                expected_watermark: before.seq(),
                expected_last_turn: record.turn_id,
                user_turns: 1,
            })
            .await,
        Err(AgentServiceError::Session(SessionError::RecoveryRequired))
    ));
}

const RECEIPT_CRASH_CHILD: &str = "agent::service::submission::tests::receipt_recovery_crash_child";
const RECEIPT_CRASH_MARKER: &str = ".receipt-recovery-fixture";

fn receipt_crash_store(path: &std::path::Path) -> crate::storage::AppStore {
    let mut sql = Connection::open(path.join("receipt.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &path.join("data-log")).unwrap();
    crate::storage::AppStore::for_test_database(db, path.join("objects"))
}

#[tokio::test]
async fn task_settlement_is_written_only_after_owner_cleanup_and_survives_reopen() {
    use crate::data_interface::{SqlImmediateStatus, TaskSettlement};
    let root = tempfile::tempdir().unwrap();
    let store = receipt_crash_store(root.path());
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    service.create(id).await.unwrap();
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let (_, _, _, example) = setup("settlement").await;
    let record = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), example.input)
        .await
        .unwrap();
    let intent = TaskSettlement {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        turn_id: record.turn_id,
    };
    let (release, wait) = oneshot::channel();
    let worker = service.clone();
    let admitted = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                wait.await.unwrap();
                finish(&worker, &record).await;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("original admission")
    };
    let run_id = owned.task.run_id();
    assert!(
        !store
            .shared_agent_connection()
            .inspect_task_run_settlement(*id.as_uuid(), record.turn_id, run_id)
            .unwrap()
    );
    assert!(
        store
            .shared_agent_connection()
            .record_task_settlement(&intent, run_id)
            .is_err()
    );
    assert!(matches!(
        store
            .shared_agent_connection()
            .task_settlement_status(&intent)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    release.send(()).unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert!(matches!(
        store
            .shared_agent_connection()
            .task_settlement_status(&intent)
            .unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    service.shutdown().await.unwrap();
    drop(owned);
    drop(submissions);
    drop(service);
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    let reopened = receipt_crash_store(root.path());
    assert!(
        reopened
            .shared_agent_connection()
            .inspect_task_run_settlement(*id.as_uuid(), record.turn_id, run_id)
            .unwrap()
    );
    assert!(
        !reopened
            .shared_agent_connection()
            .inspect_task_run_settlement(*id.as_uuid(), record.turn_id, Uuid::new_v4())
            .unwrap()
    );
    assert!(
        !reopened
            .shared_agent_connection()
            .inspect_task_run_settlement(Uuid::new_v4(), record.turn_id, run_id)
            .unwrap()
    );
    assert!(
        !reopened
            .shared_agent_connection()
            .inspect_task_run_settlement(*id.as_uuid(), Uuid::new_v4(), run_id)
            .unwrap()
    );
    assert!(matches!(
        reopened
            .shared_agent_connection()
            .task_settlement_status(&intent)
            .unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    assert!(
        reopened
            .shared_agent_connection()
            .record_task_settlement(&intent, Uuid::new_v4())
            .is_err()
    );
    // Simulate an older version that recorded only the request-level proof.
    reopened
        .connection()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where kind='agent.task.run_settled'",
            [],
        )
        .unwrap();
    assert!(
        !reopened
            .shared_agent_connection()
            .inspect_task_run_settlement(*id.as_uuid(), record.turn_id, run_id)
            .unwrap()
    );
    assert!(
        reopened
            .shared_agent_connection()
            .record_task_settlement(&intent, run_id)
            .is_err()
    );
    let wrong = TaskSettlement {
        turn_id: Uuid::new_v4(),
        ..intent
    };
    assert!(
        reopened
            .shared_agent_connection()
            .record_task_settlement(&wrong, run_id)
            .is_err()
    );
    assert!(matches!(
        reopened
            .shared_agent_connection()
            .task_settlement_status(&wrong)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    reopened.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
async fn failed_settlement_never_turns_terminal_history_into_owner_proof() {
    use crate::data_interface::{SqlImmediateStatus, TaskSettlement};
    let root = tempfile::tempdir().unwrap();
    let store = receipt_crash_store(root.path());
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    service.create(id).await.unwrap();
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let (_, _, _, example) = setup("settlement failure").await;
    let record = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), example.input)
        .await
        .unwrap();
    let intent = TaskSettlement {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        turn_id: record.turn_id,
    };
    store.connection().unwrap().execute_batch("create trigger fixture_reject_task_settlement before insert on data_sql_operation_receipts when new.kind='agent.task.run_settled' begin select raise(abort, 'synthetic receipt failure'); end;").unwrap();
    let worker = service.clone();
    let admitted = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                finish(&worker, &record).await;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("original admission")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let observed = crate::agent::turn_commands::observe_task_with_persistence(
        &service,
        crate::agent::turn_commands::TaskObservationRequest {
            conversation_id: *id.as_uuid(),
            turn_id: record.turn_id,
            run_id: owned.task.run_id(),
        },
    )
    .await
    .unwrap();
    let response = serde_json::to_value(observed).unwrap();
    let detail = &response["status"]["diagnostic"];
    assert_eq!(response["status"]["completion"], "recoveryRequired");
    assert_eq!(detail["code"], "DATA_SQL_UNAVAILABLE");
    assert_eq!(detail["phase"], "receipt");
    assert_eq!(detail["safeCause"], "sql_registration_failed");
    assert_eq!(detail["requestId"], record.request_id.to_string());
    assert!(detail["operationId"].as_str().is_some());
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    assert!(!response.to_string().contains("synthetic receipt failure"));
    assert_eq!(
        service
            .open(id)
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(matches!(
        store
            .shared_agent_connection()
            .task_settlement_status(&intent)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    service.shutdown().await.unwrap();
    drop(owned);
    drop(submissions);
    drop(service);
    store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "isolated synthetic process, launched by receipt_recovery_survives_process_exit"]
async fn receipt_recovery_crash_child() {
    let Some(path) = std::env::var_os("RESEARCH_RECEIPT_CRASH_PATH") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    assert!(path.is_absolute());
    assert_eq!(
        std::fs::read(path.join(RECEIPT_CRASH_MARKER)).unwrap(),
        b"receipt-recovery-v1"
    );
    let cut = std::env::var("RESEARCH_RECEIPT_CRASH_CUT").unwrap();
    assert!(["call", "pending", "result", "confirmed"].contains(&cut.as_str()));
    let store = receipt_crash_store(&path);
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let id = ConversationId::from_uuid(Uuid::from_u128(1));
    service.create(id).await.unwrap();
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let (_, _, _, example) = setup("receipt crash").await;
    let record = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), example.input)
        .await
        .unwrap();
    let worker = service.clone();
    let admission = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                let call_id = Uuid::from_u128(2);
                let result_id = Uuid::from_u128(3);
                let payloads = [
                    EventPayload::ItemAccepted {
                        turn_id: record.turn_id,
                        item_id: Uuid::from_u128(4),
                        kind: ItemKind::ToolCall {
                            call_id,
                            name: "effect".into(),
                            provider_call_id: None,
                            namespace: None,
                        },
                        text: "{}".into(),
                        phase: None,
                        source: None,
                    },
                    EventPayload::ToolReceiptPending {
                        turn_id: record.turn_id,
                        call_id,
                    },
                    EventPayload::ItemStarted {
                        turn_id: record.turn_id,
                        item_id: result_id,
                        kind: ItemKind::ToolResult {
                            call_id,
                            is_error: Some(false),
                        },
                    },
                    EventPayload::ItemCompleted {
                        turn_id: record.turn_id,
                        item_id: result_id,
                    },
                    EventPayload::ToolReceiptConfirmed {
                        turn_id: record.turn_id,
                        call_id,
                    },
                ];
                let count = match cut.as_str() {
                    "call" => 1,
                    "pending" => 2,
                    "result" => 4,
                    "confirmed" => 5,
                    _ => unreachable!(),
                };
                for payload in payloads.into_iter().take(count) {
                    worker
                        .commit_payload(id, record.request_id, payload)
                        .await
                        .map_err(|_| RunError::EventSink)?;
                }
                // Every selected event was acknowledged; bypass task reconciliation and all Drop handlers.
                std::process::exit(0);
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("initial admission")
    };
    owned.task.wait().await;
    panic!("child did not exit at the selected boundary");
}

#[tokio::test]
async fn receipt_recovery_survives_process_exit() {
    for cut in ["call", "pending", "result", "confirmed"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(RECEIPT_CRASH_MARKER),
            b"receipt-recovery-v1",
        )
        .unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                RECEIPT_CRASH_CHILD,
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RESEARCH_RECEIPT_CRASH_PATH", root.path())
            .env("RESEARCH_RECEIPT_CRASH_CUT", cut)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{cut}: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let store = receipt_crash_store(root.path());
        let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
        let id = ConversationId::from_uuid(Uuid::from_u128(1));
        let history = service.open(id).await.unwrap();
        assert_eq!(
            history.has_unknown_tool_outcome(),
            cut != "confirmed",
            "{cut}"
        );
        let last = *history.turn_order().last().unwrap();
        let original_request: String = store
            .connection()
            .unwrap()
            .query_row(
                "select request_id from agent_submissions where conversation_id=?1",
                [id.as_uuid().to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let settlement = crate::data_interface::TaskSettlement {
            conversation_id: *id.as_uuid(),
            request_id: Uuid::parse_str(&original_request).unwrap(),
            turn_id: last,
        };
        assert!(
            matches!(
                store
                    .shared_agent_connection()
                    .task_settlement_status(&settlement)
                    .unwrap(),
                crate::data_interface::SqlImmediateStatus::NoSqlReceipt { .. }
            ),
            "{cut}"
        );
        let turn = history.turn(last).unwrap();
        assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(
            turn.pending_tool_receipts.contains(&Uuid::from_u128(2)),
            cut == "pending" || cut == "result"
        );
        assert_eq!(
            turn.items.values().any(|item| item.completed
                && matches!(item.kind, ItemKind::ToolResult { call_id, .. } if call_id == Uuid::from_u128(2))),
            cut == "result" || cut == "confirmed",
            "{cut}"
        );
        if cut != "confirmed" {
            assert!(matches!(
                service
                    .rollback_conversation(super::super::ConversationRollbackRequest {
                        conversation_id: id,
                        request_id: Uuid::new_v4(),
                        expected_watermark: history.seq(),
                        expected_last_turn: last,
                        user_turns: 1,
                    })
                    .await,
                Err(AgentServiceError::Session(SessionError::RecoveryRequired))
            ));
        }
        let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
        let (_, _, _, example) = setup("next input").await;
        let next = submissions
            .put(*id.as_uuid(), Uuid::new_v4(), example.input)
            .await
            .unwrap();
        let called = Arc::new(AtomicUsize::new(0));
        let called_by_worker = called.clone();
        let admission = service
            .start_submission(
                &submissions,
                id,
                next.request_id,
                move |_, _, _| async move {
                    called_by_worker.fetch_add(1, Ordering::SeqCst);
                    Err(RunError::EventSink)
                },
            )
            .await;
        if cut == "confirmed" {
            let Ok(SubmissionAdmission::Started(owned)) = admission else {
                panic!("{cut}: next submission not admitted")
            };
            owned.task.wait().await;
            assert_eq!(called.load(Ordering::SeqCst), 1);
        } else {
            assert!(
                matches!(
                    admission,
                    Err(SubmissionRunError::Session(AgentServiceError::Session(
                        SessionError::RecoveryRequired
                    )))
                ),
                "{cut}"
            );
            assert_eq!(called.load(Ordering::SeqCst), 0);
        }
        service.shutdown().await.unwrap();
        drop(submissions);
        drop(service);
        store.shared_agent_connection().shutdown().unwrap();
    }
}

#[tokio::test]
async fn unknown_tool_outcome_blocks_new_submission_and_rollback_after_database_reopen() {
    use research_agent_core::protocol::FailureCategory;
    let (db, service, submissions, record) = setup("external effect").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let request_id = record.request_id;
    let previous_turn = record.turn_id;
    let next_input = record.input.clone();
    let worker = service.clone();
    let admitted = service
        .start_submission(
            &submissions,
            id,
            request_id,
            move |record, _, _| async move {
                for payload in [
                    EventPayload::ItemAccepted {
                        turn_id: record.turn_id,
                        item_id: Uuid::new_v4(),
                        kind: ItemKind::ToolCall {
                            call_id: Uuid::new_v4(),
                            name: "external_effect".into(),
                            provider_call_id: Some("provider-call".into()),
                            namespace: None,
                        },
                        text: "{}".into(),
                        phase: None,
                        source: None,
                    },
                    EventPayload::TurnFailed {
                        turn_id: record.turn_id,
                        error: RunError::ToolOutcomeUnknown.to_string(),
                        category: FailureCategory::Recovery,
                    },
                    EventPayload::TurnFinished {
                        turn_id: record.turn_id,
                        outcome: TurnOutcome::Failed,
                    },
                ] {
                    worker
                        .commit_payload(id, record.request_id, payload)
                        .await
                        .map_err(|_| RunError::EventSink)?;
                }
                Err(RunError::ToolOutcomeUnknown)
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("initial admission")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let before = service.open(id).await.unwrap();
    assert!(before.has_unknown_tool_outcome());
    let executions = Arc::new(AtomicUsize::new(0));
    let next = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), next_input.clone())
        .await
        .unwrap();
    let called = executions.clone();
    let blocked = service
        .start_submission(
            &submissions,
            id,
            next.request_id,
            move |_, _, _| async move {
                called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(matches!(
        blocked,
        Err(SubmissionRunError::Session(AgentServiceError::Session(
            SessionError::RecoveryRequired
        )))
    ));
    assert_eq!(service.open(id).await.unwrap(), before);
    service.shutdown().await.unwrap();

    let reopened = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db, "test".into()).unwrap(),
    )));
    assert_eq!(reopened.open(id).await.unwrap(), before);
    let next = submissions
        .put(*id.as_uuid(), Uuid::new_v4(), next_input)
        .await
        .unwrap();
    let called = executions.clone();
    let blocked = reopened
        .start_submission(
            &submissions,
            id,
            next.request_id,
            move |_, _, _| async move {
                called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(matches!(
        blocked,
        Err(SubmissionRunError::Session(AgentServiceError::Session(
            SessionError::RecoveryRequired
        )))
    ));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(matches!(
        reopened
            .rollback_conversation(super::super::ConversationRollbackRequest {
                conversation_id: id,
                request_id: Uuid::new_v4(),
                expected_watermark: before.seq(),
                expected_last_turn: previous_turn,
                user_turns: 1,
            })
            .await,
        Err(AgentServiceError::Session(SessionError::RecoveryRequired))
    ));
    assert_eq!(reopened.open(id).await.unwrap(), before);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn admission_lookup_preserves_saved_identity_and_never_starts_work() {
    let (_, service, submissions, record) = setup("question").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    assert!(
        service
            .existing_submission(&submissions, &record)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(service.open(id).await.unwrap().seq(), 0);
    let mut forged = record.clone();
    forged.input.text = "changed input".into();
    assert!(matches!(
        service.existing_submission(&submissions, &forged).await,
        Err(SubmissionRunError::Input(SubmissionError::Conflict))
    ));
    let worker = service.clone();
    let started = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                finish(&worker, &record).await;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = started else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert!(
        matches!(service.existing_submission(&submissions, &record).await.unwrap(),
        Some(SubmissionAdmission::AlreadyStarted { turn_id, outcome: Some(TurnOutcome::Completed) })
            if turn_id == record.turn_id)
    );
    // Even after admission, a different saved input must not obtain the receipt.
    forged = record.clone();
    forged.context_snapshot_id = Uuid::new_v4();
    assert!(matches!(
        service.existing_submission(&submissions, &forged).await,
        Err(SubmissionRunError::Input(SubmissionError::Conflict))
    ));
    assert_eq!(service.open(id).await.unwrap().seq(), 5);
}

#[tokio::test]
async fn input_is_durable_before_work_and_retry_after_completion_does_not_run_again() {
    let text = "中".repeat(70_000);
    let (_, service, submissions, record) = setup(&text).await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let worker = service.clone();
    let expected = text.clone();
    let started = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, history, _| async move {
                assert_eq!(history.seq(), 0);
                let projection = worker.open(id).await.unwrap();
                let turn = projection.turn(record.turn_id).unwrap();
                assert_eq!(turn.context_snapshot_id, record.context_snapshot_id);
                assert_eq!(turn.items[&turn.item_order[0]].text, expected);
                assert_eq!(
                    turn.items
                        .values()
                        .filter(|item| matches!(item.kind, ItemKind::UserMessage))
                        .count(),
                    1,
                    "even chunked input is exactly one durable user message"
                );
                assert!(turn.items[&turn.item_order[0]].completed);
                assert_eq!(worker.read_since(id, 0, 50).await.unwrap().durable_count, 5);
                finish(&worker, &record).await;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = started else {
        panic!()
    };
    assert_eq!(owned.turn_id, record.turn_id);
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let retry = service
        .start_submission(&submissions, id, record.request_id, |_, _, _| async {
            panic!("duplicate work")
        })
        .await
        .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    assert_eq!(service.open(id).await.unwrap().seq(), 6);
}

#[tokio::test]
async fn active_retry_keeps_original_task_and_panic_is_not_reexecuted_after_reopen() {
    let (db, service, submissions, record) = setup("question").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let (release, wait) = oneshot::channel();
    let started = service
        .start_submission(&submissions, id, record.request_id, |_, _, _| async move {
            wait.await.unwrap();
            panic!("synthetic crash after input commit")
        })
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = started else {
        panic!()
    };
    let retry = service
        .start_submission(&submissions, id, record.request_id, |_, _, _| async {
            panic!("duplicate")
        })
        .await
        .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted { outcome: None, .. }
    ));
    release.send(()).unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    service.shutdown().await.unwrap();
    drop(service);
    let reopened = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db, "test".into()).unwrap(),
    )));
    let retry = reopened
        .start_submission(&submissions, id, record.request_id, |_, _, _| async {
            panic!("restart duplicated effects")
        })
        .await
        .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Interrupted),
            ..
        }
    ));
}

#[tokio::test]
async fn rejected_initial_transaction_never_reaches_external_work() {
    let (db, service, submissions, record) = setup("question").await;
    db.lock().unwrap().execute_batch("create trigger reject_input before insert on agent_items begin select raise(abort, 'fixture'); end;").unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let result = service
        .start_submission(
            &submissions,
            ConversationId::from_uuid(record.conversation_id),
            record.request_id,
            move |_, _, _| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(matches!(result, Err(SubmissionRunError::Session(_))));
    service.tasks().shutdown().await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        service
            .read_since(ConversationId::from_uuid(record.conversation_id), 0, 10)
            .await
            .unwrap()
            .durable_count,
        0
    );
    assert_eq!(
        submissions
            .get(record.conversation_id, record.request_id)
            .await
            .unwrap(),
        record
    );
}

#[tokio::test]
async fn dropping_the_admission_receipt_does_not_abandon_owned_work() {
    let (_, service, submissions, record) = setup("question").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let worker = service.clone();
    let (release, wait) = oneshot::channel();
    let (done, completed) = oneshot::channel();
    let admission = service
        .start_submission(
            &submissions,
            id,
            record.request_id,
            move |record, _, _| async move {
                wait.await.unwrap();
                finish(&worker, &record).await;
                done.send(()).unwrap();
                Ok(())
            },
        )
        .await
        .unwrap();
    assert!(matches!(admission, SubmissionAdmission::Started(_)));
    drop(admission);
    release.send(()).unwrap();
    completed.await.unwrap();
    let results = service.tasks().shutdown().await.unwrap();
    assert_eq!(results, vec![(id, TaskCompletion::Completed)]);
    assert_eq!(
        service
            .open(id)
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test]
async fn concurrent_first_admission_never_invokes_both_operations() {
    let (_, service, submissions, record) = setup("question").await;
    let id = ConversationId::from_uuid(record.conversation_id);
    let count = Arc::new(AtomicUsize::new(0));
    let run = || {
        let worker = service.clone();
        let count = count.clone();
        move |record: SubmissionRecord, _, _| async move {
            count.fetch_add(1, Ordering::SeqCst);
            finish(&worker, &record).await;
            Ok(())
        }
    };
    let (first, second) = tokio::join!(
        service.start_submission(&submissions, id, record.request_id, run()),
        service.start_submission(&submissions, id, record.request_id, run())
    );
    let mut started = 0;
    for admission in [first, second] {
        match admission {
            Ok(SubmissionAdmission::Started(owned)) => {
                started += 1;
                assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
            }
            Ok(SubmissionAdmission::AlreadyStarted { .. })
            | Err(SubmissionRunError::Task(TaskRegistryError::Active)) => {}
            _ => panic!("unexpected admission failure"),
        }
    }
    assert_eq!(started, 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
