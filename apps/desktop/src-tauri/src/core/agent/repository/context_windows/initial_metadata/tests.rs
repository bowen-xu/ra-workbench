use super::*;
use crate::agent::{cancellation::AgentCancellation, repository::context_windows::tests::fixture};

fn setup(mut db: Connection) -> (Connection, Snapshot) {
    let checkpoint = fixture(&mut db);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let root = identity::read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7())
        .unwrap()
        .first;
    let research_agent_core::model::ModelMessage::Developer(body) = (TokenBudgetContext {
        conversation_id: checkpoint.conversation_id,
        first_window_id: root,
        previous_window_id: None,
        window_id: root,
        thread_hint: Some("original hint".into()),
    })
    .into_message() else {
        panic!("developer role expected")
    };
    (
        db,
        Snapshot {
            conversation: checkpoint.conversation_id,
            window: root,
            boundary: 1,
            turn: checkpoint.turn_id,
            body,
        },
    )
}

#[test]
fn snapshot_survives_database_reopen_and_future_cutoff_does_not_expose_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("initial.sqlite");
    let (mut db, snapshot) = setup(Connection::open(&path).unwrap());
    assert_eq!(
        record(&mut db, &snapshot, &AgentCancellation::default()).unwrap(),
        snapshot.body
    );
    assert!(
        read(&db, snapshot.conversation, snapshot.window, 0)
            .unwrap()
            .is_none()
    );
    drop(db);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        read(&db, snapshot.conversation, snapshot.window, 1)
            .unwrap()
            .unwrap()
            .body,
        snapshot.body
    );
    assert!(
        db.execute(
            "update agent_initial_window_metadata set body='changed'",
            []
        )
        .is_err()
    );
}

#[test]
fn canceled_or_failed_commit_leaves_no_snapshot_and_retry_is_immutable() {
    let (mut db, mut snapshot) = setup(Connection::open_in_memory().unwrap());
    let signal = AgentCancellation::default();
    signal.cancel();
    assert_eq!(record(&mut db, &snapshot, &signal), Err(Error::Canceled));
    assert!(
        read(&db, snapshot.conversation, snapshot.window, 1)
            .unwrap()
            .is_none()
    );
    db.execute_batch("create trigger fail_initial_snapshot before insert on agent_initial_window_metadata begin select raise(abort,'fixture write failure'); end;").unwrap();
    assert_eq!(
        record(&mut db, &snapshot, &AgentCancellation::default()),
        Err(Error::Storage)
    );
    assert!(
        read(&db, snapshot.conversation, snapshot.window, 1)
            .unwrap()
            .is_none()
    );
    db.execute_batch("drop trigger fail_initial_snapshot")
        .unwrap();
    let original = record(&mut db, &snapshot, &AgentCancellation::default()).unwrap();
    snapshot.body = snapshot.body.replace("original hint", "later hint");
    assert_eq!(
        record(&mut db, &snapshot, &AgentCancellation::default()).unwrap(),
        original
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_initial_window_metadata",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn rollback_skips_revoked_snapshot_and_new_turn_can_publish_fresh_hint() {
    use crate::agent::repository::context_windows::tests::append;
    use research_agent_core::protocol::{EventPayload, ItemKind, TaskKind, TurnOutcome};
    let (mut db, mut snapshot) = setup(Connection::open_in_memory().unwrap());
    record(&mut db, &snapshot, &AgentCancellation::default()).unwrap();
    let user = Uuid::new_v4();
    for (index, event) in [
        EventPayload::ItemStarted {
            turn_id: snapshot.turn,
            item_id: user,
            kind: ItemKind::UserMessage,
        },
        EventPayload::ItemCompleted {
            turn_id: snapshot.turn,
            item_id: user,
        },
        EventPayload::TurnFinished {
            turn_id: snapshot.turn,
            outcome: TurnOutcome::Completed,
        },
        EventPayload::ConversationRolledBack {
            turn_id: snapshot.turn,
            user_turns: 1,
        },
    ]
    .into_iter()
    .enumerate()
    {
        append(&mut db, snapshot.conversation, 1 + index as u64, event);
    }
    assert!(
        read(&db, snapshot.conversation, snapshot.window, 5)
            .unwrap()
            .is_none()
    );
    snapshot.turn = Uuid::new_v4();
    snapshot.boundary = 6;
    snapshot.body = snapshot.body.replace("original hint", "fresh hint");
    append(
        &mut db,
        snapshot.conversation,
        5,
        EventPayload::TurnStarted {
            turn_id: snapshot.turn,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "model".into(),
                revision: None,
            },
        },
    );
    assert_eq!(
        record(&mut db, &snapshot, &AgentCancellation::default()).unwrap(),
        snapshot.body
    );
    assert_eq!(
        read(&db, snapshot.conversation, snapshot.window, 6)
            .unwrap()
            .unwrap()
            .body,
        snapshot.body
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_initial_window_metadata",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}
