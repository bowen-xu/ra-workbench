use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind, TurnOutcome},
};
use rusqlite::TransactionBehavior;

fn fixture(path: &std::path::Path) -> (Connection, Admission) {
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let conversation = ConversationId::from_uuid(Uuid::new_v4());
    super::super::create_conversation(&mut db, conversation, "stop audit").unwrap();
    let admission = Admission {
        invocation_id: Uuid::new_v4(),
        conversation_id: *conversation.as_uuid(),
        turn_id: Uuid::new_v4(),
        event: StopEvent::Stop,
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: Some("v1".into()),
        },
        handlers: vec![
            (Uuid::new_v4(), Uuid::new_v4()),
            (Uuid::new_v4(), Uuid::new_v4()),
        ],
    };
    let event = AgentEvent::new(
        conversation,
        1,
        1,
        Uuid::new_v4(),
        EventPayload::TurnStarted {
            turn_id: admission.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: admission.model.clone(),
        },
    )
    .unwrap();
    super::super::append_batch(&mut db, conversation, Uuid::new_v4(), 0, &[event]).unwrap();
    (db, admission)
}
fn completed(a: &Admission) -> Finished {
    Finished {
        canceled: true,
        outcomes: vec![
            (
                a.handlers[0].0,
                a.handlers[0].1,
                Outcome::Completed {
                    status: Status::Blocked {
                        feedback: "retry & verify".into(),
                    },
                    system_message: Some("notice".into()),
                },
            ),
            (a.handlers[1].0, a.handlers[1].1, Outcome::Canceled),
        ],
    }
}
fn admit_commit(db: &mut Connection, admission: &Admission) {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    admit(&tx, admission, 10).unwrap();
    tx.commit().unwrap();
}

#[test]
fn incomplete_admission_and_completed_outputs_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("stop.sqlite");
    let (mut db, a) = fixture(&path);
    admit_commit(&mut db, &a);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let tx = db.transaction().unwrap();
    let saved = read(&tx, a.conversation_id, a.invocation_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.boundary_seq, 1);
    assert_eq!(saved.admission.handlers, a.handlers);
    assert!(saved.finished.is_none());
    tx.commit().unwrap();
    let expected = completed(&a);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    finish(&tx, &a, &expected, 20).unwrap();
    tx.commit().unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let tx = db.transaction().unwrap();
    let saved = read(&tx, a.conversation_id, a.invocation_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.finished.unwrap(), expected);
    tx.commit().unwrap();
}

#[test]
fn invalid_identity_or_partial_result_never_commits_a_finish() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, a) = fixture(&root.path().join("stop.sqlite"));
    admit_commit(&mut db, &a);
    let mut bad = completed(&a);
    bad.outcomes.swap(0, 1);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(finish(&tx, &a, &bad, 20).is_err());
    tx.rollback().unwrap();
    db.execute_batch("create trigger synthetic_reject_stop before insert on agent_stop_hook_results when new.ordinal=1 begin select raise(abort,'synthetic failure'); end;").unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(finish(&tx, &a, &completed(&a), 20).is_err());
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_stop_hook_results", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from agent_stop_hook_finishes", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger synthetic_reject_stop;")
        .unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    finish(&tx, &a, &completed(&a), 20).unwrap();
    tx.commit().unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(finish(&tx, &a, &completed(&a), 21).is_err());
    tx.rollback().unwrap();
    assert!(
        db.execute("update agent_stop_hook_results set outcome_json='{}'", [])
            .is_err()
    );
    assert!(
        db.execute("delete from agent_stop_hook_handlers", [])
            .is_err()
    );
    assert!(db.execute("insert into agent_stop_hook_handlers(invocation_id,ordinal,handler_id,run_id) values(?1,2,?2,?3)",params![a.invocation_id.to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string()]).is_err());
}

#[test]
fn admission_rejects_wrong_model_turn_and_finished_conversation() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, a) = fixture(&root.path().join("stop.sqlite"));
    let mut wrong = a.clone();
    wrong.model.revision = None;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(admit(&tx, &wrong, 10).is_err());
    wrong = a.clone();
    wrong.turn_id = Uuid::new_v4();
    assert!(admit(&tx, &wrong, 10).is_err());
    tx.rollback().unwrap();
    let conversation = ConversationId::from_uuid(a.conversation_id);
    let event = AgentEvent::new(
        conversation,
        2,
        2,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: a.turn_id,
            outcome: TurnOutcome::Completed,
        },
    )
    .unwrap();
    super::super::append_batch(&mut db, conversation, Uuid::new_v4(), 1, &[event]).unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(admit(&tx, &a, 10).is_err());
    tx.rollback().unwrap();
}

#[test]
fn active_read_checks_original_conversation_and_turn_in_same_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, a) = fixture(&root.path().join("stop.sqlite"));
    admit_commit(&mut db, &a);
    let tx = db.transaction().unwrap();
    assert!(
        read_active(&tx, Uuid::new_v4(), a.invocation_id)
            .unwrap()
            .is_none()
    );
    let saved = read_active(&tx, a.conversation_id, a.invocation_id)
        .unwrap()
        .unwrap();
    assert!(saved.finished.is_none());
    tx.commit().unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    finish(&tx, &a, &completed(&a), 20).unwrap();
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        read_active(&tx, a.conversation_id, a.invocation_id)
            .unwrap()
            .unwrap()
            .finished,
        Some(completed(&a))
    );
    tx.commit().unwrap();
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [a.conversation_id.to_string()],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert!(read_active(&tx, a.conversation_id, a.invocation_id).is_err());
    tx.commit().unwrap();
    db.execute(
        "update agent_conversations set archived=0 where id=?1",
        [a.conversation_id.to_string()],
    )
    .unwrap();
    let conversation = ConversationId::from_uuid(a.conversation_id);
    let event = AgentEvent::new(
        conversation,
        2,
        2,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: a.turn_id,
            outcome: TurnOutcome::Completed,
        },
    )
    .unwrap();
    super::super::append_batch(&mut db, conversation, Uuid::new_v4(), 1, &[event]).unwrap();
    let tx = db.transaction().unwrap();
    assert!(read_active(&tx, a.conversation_id, a.invocation_id).is_err());
    // The audit remains available for inspection, but not for continuation.
    assert!(
        read(&tx, a.conversation_id, a.invocation_id)
            .unwrap()
            .unwrap()
            .finished
            .is_some()
    );
}

#[test]
fn conversation_deletion_cascades_immutable_audit() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, a) = fixture(&root.path().join("stop.sqlite"));
    admit_commit(&mut db, &a);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    finish(&tx, &a, &completed(&a), 20).unwrap();
    tx.commit().unwrap();
    db.execute(
        "delete from agent_conversations where id=?1",
        [a.conversation_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_stop_hook_admissions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from agent_stop_hook_results", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn managed_data_interface_commits_and_reopens_stop_facts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("managed.sqlite");
    let (sql, a) = fixture(&path);
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let store = SqliteStopHookAudit::new(db.clone());
    store.admit(a.clone(), 10).await.unwrap();
    assert!(
        store
            .read(Uuid::new_v4(), a.invocation_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .read(a.conversation_id, a.invocation_id)
            .await
            .unwrap()
            .unwrap()
            .finished
            .is_none()
    );
    db.lock().unwrap().execute_batch("create trigger synthetic_reject_stop before insert on agent_stop_hook_results when new.ordinal=1 begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(store.finish(a.clone(), completed(&a), 20).await.is_err());
    assert!(
        store
            .read(a.conversation_id, a.invocation_id)
            .await
            .unwrap()
            .unwrap()
            .finished
            .is_none()
    );
    db.lock()
        .unwrap()
        .execute_batch("drop trigger synthetic_reject_stop;")
        .unwrap();
    store.finish(a.clone(), completed(&a), 20).await.unwrap();
    drop(store);
    drop(db);
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let store = SqliteStopHookAudit::new(db);
    assert_eq!(
        store
            .read_active(a.conversation_id, a.invocation_id)
            .await
            .unwrap()
            .unwrap()
            .finished
            .unwrap(),
        completed(&a)
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn reopened_committed_feedback_reconstructs_hook_prompt_with_run_identity() {
    use crate::agent::compaction_hook_config::native::StopResolution;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("prompt.sqlite");
    let (sql, a) = fixture(&path);
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let store = SqliteStopHookAudit::new(db);
    store.admit(a.clone(), 10).await.unwrap();
    let mut result = completed(&a);
    result.canceled = false;
    result.outcomes[1].2 = Outcome::Completed {
        status: Status::Completed,
        system_message: None,
    };
    store.finish(a.clone(), result, 20).await.unwrap();
    drop(store);
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let store = SqliteStopHookAudit::new(db);
    let saved = store
        .read(a.conversation_id, a.invocation_id)
        .await
        .unwrap()
        .unwrap();
    let Some(StopResolution::FollowUp { parts }) = StopResolution::from_recorded(&saved) else {
        panic!("committed prompt must survive reopen");
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].run_id, a.handlers[0].1);
    assert!(parts[0].input_text.contains("retry &amp; verify"));
}
