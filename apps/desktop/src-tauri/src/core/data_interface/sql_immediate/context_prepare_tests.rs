use super::*;
use crate::storage::AppStore;
use rusqlite::Connection;

struct Fixture {
    root: tempfile::TempDir,
    database: crate::data_interface::SharedDatabase,
    store: AppStore,
    intent: ContextPrepareIntent,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("context.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic')",
            [conversation.to_string()],
        )
        .unwrap();
        let database = Database::open(sql, &root.path().join("log")).unwrap();
        let store = AppStore::for_test_database(database.clone(), root.path().join("objects"));
        let request = serde_json::from_value(serde_json::json!({"conversationId":conversation,"requestId":Uuid::new_v4(),"text":"Synthetic question","resources":[],"model":null,"retrieval":null,"queue":null})).unwrap();
        Self {
            root,
            database,
            store,
            intent: ContextPrepareIntent {
                request,
                expected_epoch: None,
            },
        }
    }
    fn source(&self) -> ContextPrepareSource {
        ContextPrepareSource {
            store: self.store.clone(),
            model: (
                ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
                None,
                None,
            ),
        }
    }
}

#[test]
fn independent_prepare_commits_receipt_with_frozen_result_and_replays_exact_intent() {
    let f = Fixture::new();
    let before = f.database.lock().unwrap().total_changes();
    match f.database.context_prepare_status(&f.intent).unwrap() {
        ContextPrepareStatus::Unconfirmed(detail) => {
            assert_eq!(detail.operation_id, f.intent.request.request_id);
            assert!(detail.epoch.is_some());
            assert_eq!(detail.safe_cause, "context_prepare_worker_unconfirmed");
        }
        _ => panic!("missing receipt cannot prove a commit"),
    }
    assert_eq!(f.database.lock().unwrap().total_changes(), before);
    assert!(
        f.database
            .replay_context_prepare(&f.intent)
            .unwrap()
            .is_none()
    );
    let (first, receipt) = f
        .database
        .prepare_context_owned(&f.intent, f.source())
        .unwrap();
    assert!(!receipt.replayed);
    let (replayed, proof) = f
        .database
        .replay_context_prepare(&f.intent)
        .unwrap()
        .unwrap();
    assert!(proof.replayed);
    assert_eq!(proof.epoch, receipt.epoch);
    assert_eq!(proof.sql_fence.position, receipt.sql_fence.position);
    assert_eq!(first.turn_id, replayed.turn_id);
    assert_eq!(first.input, replayed.input);
    match f.database.context_prepare_status(&f.intent).unwrap() {
        ContextPrepareStatus::Committed {
            preview,
            receipt: observed,
        } => {
            assert_eq!(preview.turn_id, first.turn_id);
            assert_eq!(observed.epoch, receipt.epoch);
            assert_eq!(observed.sql_fence.position, receipt.sql_fence.position);
        }
        _ => panic!("exact receipt must be observable"),
    }
    let mut changed = ContextPrepareIntent {
        request: f.intent.request.clone(),
        expected_epoch: None,
    };
    changed.request.text.push('!');
    assert!(matches!(
        f.database.replay_context_prepare(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        f.database.context_prepare_status(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let mut wrong = ContextPrepareIntent {
        request: f.intent.request.clone(),
        expected_epoch: Some(Uuid::new_v4()),
    };
    assert!(matches!(
        f.database.replay_context_prepare(&wrong),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        f.database.context_prepare_status(&wrong),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    wrong.expected_epoch = Some(receipt.epoch);
    assert!(matches!(
        f.database.replay_context_prepare(&wrong),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}

#[test]
fn independent_prepare_refuses_sources_from_another_data_owner() {
    let first = Fixture::new();
    let second = Fixture::new();
    assert!(matches!(
        first
            .database
            .prepare_context_owned(&first.intent, second.source()),
        Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput))
    ));
    assert!(
        first
            .database
            .replay_context_prepare(&first.intent)
            .unwrap()
            .is_none()
    );
}

#[test]
fn status_observes_a_guarded_prepare_only_in_the_original_environment() {
    let mut f = Fixture::new();
    let epoch = match f.database.context_prepare_status(&f.intent).unwrap() {
        ContextPrepareStatus::Unconfirmed(detail) => detail.epoch.unwrap(),
        _ => panic!("new request must be unconfirmed"),
    };
    f.intent.expected_epoch = Some(epoch);
    assert!(matches!(
        f.database.context_prepare_status(&f.intent).unwrap(),
        ContextPrepareStatus::Unconfirmed(_)
    ));
    let (original, proof) = f
        .database
        .prepare_context_owned(&f.intent, f.source())
        .unwrap();
    match f.database.context_prepare_status(&f.intent).unwrap() {
        ContextPrepareStatus::Committed { preview, receipt } => {
            assert_eq!(preview.turn_id, original.turn_id);
            assert_eq!(receipt.epoch, epoch);
            assert_eq!(receipt.sql_fence.position, proof.sql_fence.position);
        }
        _ => panic!("guarded receipt should be observable"),
    }
    let unguarded = ContextPrepareIntent {
        request: f.intent.request.clone(),
        expected_epoch: None,
    };
    assert!(matches!(
        f.database.context_prepare_status(&unguarded),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}

#[test]
fn replay_rejects_a_changed_frozen_submission_even_with_the_original_intent() {
    let f = Fixture::new();
    f.database
        .prepare_context_owned(&f.intent, f.source())
        .unwrap();
    {
        let db = f.database.lock().unwrap();
        db.execute("update agent_submissions set input_json=json_set(input_json,'$.model.modelId','tampered') where request_id=?1", [f.intent.request.request_id.to_string()]).unwrap();
    }
    assert!(matches!(
        f.database.context_prepare_status(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.database.lock().is_ok());
    assert!(matches!(
        f.database.replay_context_prepare(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.database.lock().is_err());
}

#[test]
fn rejected_frozen_result_proof_cannot_commit_a_submission() {
    let f = Fixture::new();
    f.database.lock().unwrap().execute_batch("create trigger ignore_context_result before update on data_sql_operation_receipts begin select raise(ignore); end;").unwrap();
    assert!(matches!(
        f.database.prepare_context_owned(&f.intent, f.source()),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.database.lock().is_err());
    let Fixture {
        root,
        database,
        store,
        intent,
    } = f;
    drop(store);
    drop(database);
    let mut sql = Connection::open(root.path().join("context.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened = Database::open(sql, &root.path().join("log")).unwrap();
    assert!(reopened.replay_context_prepare(&intent).unwrap().is_none());
}

#[test]
fn replay_rejects_changed_frozen_resource_grant() {
    let mut f = Fixture::new();
    let note = Uuid::new_v4();
    {
        let db = f.database.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic note','now','now')", [note.to_string()]).unwrap();
    }
    f.intent.request.resources.push(
        serde_json::from_value(serde_json::json!({"kind":"note","resourceId":note})).unwrap(),
    );
    let (preview, _) = f
        .database
        .prepare_context_owned(&f.intent, f.source())
        .unwrap();
    assert_eq!(preview.input.context.len(), 1);
    f.database
        .lock()
        .unwrap()
        .execute(
            "update note_pages set title='Changed note' where id=?1",
            [note.to_string()],
        )
        .unwrap();
    let (replayed, _) = f
        .database
        .replay_context_prepare(&f.intent)
        .unwrap()
        .unwrap();
    assert_eq!(replayed.input.context, preview.input.context);
    {
        let db = f.database.lock().unwrap();
        db.execute("update agent_submissions set input_json=json_set(input_json,'$.context[0].text','changed grant') where request_id=?1", [f.intent.request.request_id.to_string()]).unwrap();
    }
    assert!(matches!(
        f.database.replay_context_prepare(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
}

#[test]
fn lost_commit_ack_is_recovered_only_from_the_original_receipt_after_reopen() {
    let f = Fixture::new();
    let result = f.database.prepare_context_with_fault(
        &f.intent,
        f.source(),
        super::super::Fault::AfterCommit,
    );
    assert!(matches!(result, Err(SqlImmediateFailure::Unknown(_))));
    assert!(f.database.lock().is_err());
    match f.database.context_prepare_status(&f.intent).unwrap() {
        ContextPrepareStatus::Committed { preview, receipt } => {
            assert_eq!(preview.request_id, f.intent.request.request_id);
            assert!(receipt.replayed);
        }
        _ => panic!("committed proof must remain inspectable behind the barrier"),
    }
    let Fixture {
        root,
        database,
        store,
        intent,
    } = f;
    drop(store);
    drop(database);
    let mut sql = Connection::open(root.path().join("context.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened = Database::open(sql, &root.path().join("log")).unwrap();
    let (result, receipt) = reopened.replay_context_prepare(&intent).unwrap().unwrap();
    assert_eq!(result.request_id, intent.request.request_id);
    assert!(receipt.replayed);
}

#[test]
fn failure_before_commit_leaves_neither_frozen_submission_nor_receipt() {
    let f = Fixture::new();
    let result = f.database.prepare_context_with_fault(
        &f.intent,
        f.source(),
        super::super::Fault::BeforeCommit,
    );
    assert!(matches!(result, Err(SqlImmediateFailure::Unknown(_))));
    assert!(matches!(
        f.database.context_prepare_status(&f.intent).unwrap(),
        ContextPrepareStatus::Unconfirmed(_)
    ));
    assert!(f.database.lock().is_err());
    let Fixture {
        root,
        database,
        store,
        intent,
    } = f;
    drop(store);
    drop(database);
    let mut sql = Connection::open(root.path().join("context.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened = Database::open(sql, &root.path().join("log")).unwrap();
    assert!(reopened.replay_context_prepare(&intent).unwrap().is_none());
    let db = reopened.lock().unwrap();
    let count: u64 = db.query_row("select (select count(*) from agent_submissions where request_id=?1)+(select count(*) from data_sql_operation_receipts where operation_id=?1)", [intent.request.request_id.to_string()], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn queued_context_preparation_freezes_queue_identity_with_the_receipt() {
    use crate::agent::repository::input_queue;
    use research_agent_core::input_queue::{QueueEdit, QueuedInput};
    let mut f = Fixture::new();
    let conversation = f.intent.request.conversation_id;
    let input_id = Uuid::new_v4();
    {
        let mut db = f.database.lock().unwrap();
        input_queue::apply_checked(
            &mut db,
            conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: f.intent.request.text.clone(),
                user_input: vec![],
                origin: None,
            }),
            |_| Ok(()),
        )
        .unwrap();
    }
    f.intent.request.queue = Some(crate::agent::queue_contract::Confirmation {
        input_id,
        expected_revision: 1,
    });
    let (first, receipt) = f
        .database
        .prepare_context_owned(&f.intent, f.source())
        .unwrap();
    assert_eq!(first.queue.unwrap().expected_revision, 2);
    let (replayed, proof) = f
        .database
        .replay_context_prepare(&f.intent)
        .unwrap()
        .unwrap();
    assert_eq!(replayed.turn_id, first.turn_id);
    assert_eq!(replayed.queue.unwrap().input_id, input_id);
    assert_eq!(proof.epoch, receipt.epoch);
    let mut changed = ContextPrepareIntent {
        request: f.intent.request.clone(),
        expected_epoch: None,
    };
    changed.request.queue.as_mut().unwrap().expected_revision = 2;
    assert!(matches!(
        f.database.replay_context_prepare(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    {
        let db = f.database.lock().unwrap();
        db.execute("update agent_input_queue_previews set result_json=json_set(result_json,'$.revision',99) where request_id=?1", [f.intent.request.request_id.to_string()]).unwrap();
    }
    assert!(matches!(
        f.database.replay_context_prepare(&f.intent),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.database.lock().is_err());
}
