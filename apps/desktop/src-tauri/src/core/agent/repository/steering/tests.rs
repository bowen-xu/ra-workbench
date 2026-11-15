use super::*;
use crate::agent::repository::{append_batch, create_conversation, submissions::insert_in};
use research_agent_core::{conversation::ConversationId, protocol::*, submission::SubmissionInput};

#[path = "tests/managed.rs"]
mod managed;

fn setup(db: Connection) -> (Arc<Mutex<Connection>>, SteeringScope) {
    setup_kind(db, TaskKind::Regular)
}
fn setup_kind(mut db: Connection, task_kind: TaskKind) -> (Arc<Mutex<Connection>>, SteeringScope) {
    crate::storage_migration_tests::migrate_through(&mut db, 122);
    let conversation_id = Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(conversation_id), "test").unwrap();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "test".into(),
        revision: None,
    };
    let record = {
        let tx = db.transaction().unwrap();
        let record = insert_in(
            &tx,
            conversation_id,
            Uuid::new_v4(),
            SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                text: "original".into(),
                model: model.clone(),
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
            },
        )
        .unwrap();
        tx.commit().unwrap();
        record
    };
    let scope = SteeringScope {
        conversation_id,
        turn_id: record.turn_id,
    };
    let db = Arc::new(Mutex::new(db));
    append(
        &db,
        scope,
        vec![EventPayload::TurnStarted {
            task_kind,
            turn_id: scope.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model,
        }],
    );
    (db, scope)
}
fn append(db: &Arc<Mutex<Connection>>, scope: SteeringScope, payloads: Vec<EventPayload>) {
    let mut db = db.lock().unwrap();
    let id = ConversationId::from_uuid(scope.conversation_id);
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [scope.conversation_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(i, p)| AgentEvent::new(id, count + i as u64 + 1, 0, Uuid::new_v4(), p).unwrap())
        .collect();
    append_batch(&mut db, id, Uuid::new_v4(), count, &events).unwrap();
}
fn input(scope: SteeringScope, text: &str) -> SteeringInput {
    SteeringInput {
        skill_instructions: vec![],
        user_input: vec![],
        scope,
        input_id: Uuid::new_v4(),
        text: text.into(),
    }
}
fn consume(db: &Arc<Mutex<Connection>>, input: &SteeringInput, text: &str) {
    let turn_id = input.scope.turn_id;
    let item_id = input.input_id;
    append(
        db,
        input.scope,
        vec![
            EventPayload::ItemStarted {
                turn_id,
                item_id,
                kind: ItemKind::UserMessage,
            },
            EventPayload::TextDelta {
                turn_id,
                item_id,
                text: text.into(),
            },
            EventPayload::ItemCompleted { turn_id, item_id },
        ],
    );
}

#[tokio::test]
async fn nonsealing_pending_reads_do_not_create_gate_or_write_sql() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let deny_writes = |context: AuthContext<'_>| match context.action {
        AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    };
    db.lock().unwrap().authorizer(Some(deny_writes));
    assert!(store.pending(scope, false).await.unwrap().is_empty());
    db.lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row("select count(*) from agent_steering_gates", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    let admitted = input(scope, "later input");
    store.enqueue(admitted.clone()).await.unwrap();
    db.lock().unwrap().authorizer(Some(deny_writes));
    assert_eq!(store.pending(scope, false).await.unwrap(), vec![admitted]);
    db.lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[tokio::test]
async fn managed_nonsealing_inspection_accepts_later_input() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let root = tempfile::tempdir().unwrap();
    let (raw, scope) = setup(Connection::open(root.path().join("steer.sqlite")).unwrap());
    let mut sql = Arc::try_unwrap(raw).ok().unwrap().into_inner().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let store = SqliteSteeringStore::new(db.clone());
    {
        let guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } => {
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
    }
    assert!(store.pending(scope, false).await.unwrap().is_empty());
    {
        let guard = db.lock().unwrap();
        guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        assert_eq!(
            guard
                .query_row("select count(*) from agent_steering_gates", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
    }
    let admitted = input(scope, "accepted after inspection");
    store.enqueue(admitted.clone()).await.unwrap();
    assert_eq!(store.pending(scope, false).await.unwrap(), vec![admitted]);
}

#[tokio::test]
async fn stale_expected_turn_reports_the_actual_turn_without_admitting_input() {
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let actual_turn_id = Uuid::new_v4();
    append(
        &db,
        scope,
        vec![
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: TurnOutcome::Completed,
            },
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: actual_turn_id,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "test".into(),
                    revision: None,
                },
            },
        ],
    );
    let store = SqliteSteeringStore::new(db.clone());
    assert_eq!(
        store.enqueue(input(scope, "stale input")).await,
        Err(SteeringError::ExpectedTurnMismatch { actual_turn_id })
    );
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row("select count(*) from agent_steering_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn startup_discards_unconsumed_inputs_but_keeps_history_identity_rows() {
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let consumed = input(scope, "already committed");
    let pending = input(scope, "still waiting");
    store.enqueue(consumed.clone()).await.unwrap();
    store.enqueue(pending.clone()).await.unwrap();
    consume(&db, &consumed, &consumed.text);

    SqliteSteeringStore::discard_unconsumed_on_startup(&mut db.lock().unwrap()).unwrap();

    let count: i64 = db
        .lock()
        .unwrap()
        .query_row("select count(*) from agent_steering_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(store.pending(scope, false).await.unwrap().is_empty());
    assert_eq!(pending.text, "still waiting");
}

#[tokio::test]
async fn terminal_recovery_survives_reopen_without_replaying_consumed_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let (db, scope) = setup(Connection::open(&path).unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let consumed = input(scope, "already in history");
    let pending = input(scope, "not yet consumed");
    store.enqueue(consumed.clone()).await.unwrap();
    store.enqueue(pending.clone()).await.unwrap();
    consume(&db, &consumed, &consumed.text);
    assert_eq!(store.recoverable(scope).await, Err(SteeringError::Closed));
    append(
        &db,
        scope,
        vec![EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: TurnOutcome::Canceled,
        }],
    );
    drop(store);
    drop(db);
    let db = Arc::new(Mutex::new(Connection::open(path).unwrap()));
    let store = SqliteSteeringStore::new(db.clone());
    assert_eq!(
        store.recoverable(scope).await.unwrap(),
        vec![pending.clone()]
    );
    assert_eq!(store.recoverable(scope).await.unwrap(), vec![pending]);
    assert_eq!(
        store.pending(scope, false).await,
        Err(SteeringError::Closed)
    );
    assert_eq!(
        store
            .recoverable(SteeringScope {
                conversation_id: Uuid::new_v4(),
                ..scope
            })
            .await,
        Err(SteeringError::Closed)
    );
    assert_eq!(
        store
            .recoverable_checked(scope, |_| Err(SteeringError::Closed))
            .await,
        Err(SteeringError::Closed)
    );
    db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [scope.conversation_id.to_string()],
        )
        .unwrap();
    assert_eq!(store.recoverable(scope).await, Err(SteeringError::Closed));
}

#[tokio::test]
async fn recovery_does_not_reclassify_partial_history_as_unsent() {
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let pending = input(scope, "incomplete history");
    store.enqueue(pending.clone()).await.unwrap();
    append(
        &db,
        scope,
        vec![
            EventPayload::ItemStarted {
                turn_id: scope.turn_id,
                item_id: pending.input_id,
                kind: ItemKind::UserMessage,
            },
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: TurnOutcome::Canceled,
            },
        ],
    );
    assert_eq!(store.recoverable(scope).await, Err(SteeringError::Invalid));
}

#[tokio::test]
async fn restart_preserves_order_and_journal_consumption_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.sqlite3");
    let (db, scope) = setup(Connection::open(&path).unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let first = input(scope, "first");
    let second = input(scope, "second");
    store.enqueue(first.clone()).await.unwrap();
    store.enqueue(second.clone()).await.unwrap();
    store.enqueue(first.clone()).await.unwrap();
    assert_eq!(
        store
            .enqueue(SteeringInput {
                text: "changed".into(),
                ..first.clone()
            })
            .await,
        Err(SteeringError::Conflict)
    );
    consume(&db, &first, &first.text);
    drop(store);
    drop(db);
    let db = Arc::new(Mutex::new(Connection::open(&path).unwrap()));
    let store = SqliteSteeringStore::new(db.clone());
    assert_eq!(
        store.pending(scope, true).await.unwrap(),
        vec![second.clone()]
    );
    let third = input(scope, "third");
    store.enqueue(third.clone()).await.unwrap();
    consume(&db, &second, &second.text);
    consume(&db, &third, &third.text);
    assert!(store.pending(scope, true).await.unwrap().is_empty());
    assert_eq!(
        store.enqueue(input(scope, "late")).await,
        Err(SteeringError::Closed)
    );
    store.enqueue(first).await.unwrap();
}

#[tokio::test]
async fn racing_seal_and_enqueue_never_lose_accepted_input() {
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let other = SqliteSteeringStore::new(db);
    let item = input(scope, "racing");
    let (admitted, pending) = tokio::join!(store.enqueue(item.clone()), other.pending(scope, true));
    let pending = pending.unwrap();
    match admitted {
        Ok(()) => assert_eq!(pending, vec![item]),
        Err(SteeringError::Closed) => assert!(pending.is_empty()),
        other => panic!("unexpected admission: {other:?}"),
    }
}

#[tokio::test]
async fn wrong_scope_terminal_turn_and_conflicting_history_are_rejected() {
    let (db, scope) = setup(Connection::open_in_memory().unwrap());
    let store = SqliteSteeringStore::new(db.clone());
    let item = input(scope, "saved");
    store.enqueue(item.clone()).await.unwrap();
    assert_eq!(
        store
            .pending(
                SteeringScope {
                    conversation_id: Uuid::new_v4(),
                    ..scope
                },
                false
            )
            .await,
        Err(SteeringError::Closed)
    );
    consume(&db, &item, "different");
    assert_eq!(
        store.pending(scope, true).await,
        Err(SteeringError::Invalid)
    );
    append(
        &db,
        scope,
        vec![EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: TurnOutcome::Canceled,
        }],
    );
    assert_eq!(
        store.enqueue(input(scope, "late")).await,
        Err(SteeringError::Closed)
    );
}

#[tokio::test]
async fn compact_task_rejects_steering_before_freezing_resources_or_writing_input() {
    let (db, scope) = setup_kind(Connection::open_in_memory().unwrap(), TaskKind::Compact);
    let store = SqliteSteeringStore::new(db.clone());
    let original = input(scope, "preserve this queued instruction");
    let rejected = SteeringError::NotSteerable {
        task_kind: TaskKind::Compact,
    };
    assert_eq!(
        store
            .enqueue_with_resources(
                original.clone(),
                |_| Ok(()),
                |_, _| { panic!("a non-steerable task must not freeze selected resources") }
            )
            .await,
        Err(rejected)
    );
    assert_eq!(store.pending(scope, false).await, Err(rejected));
    // Repeating the rejected identity cannot turn rejection into admission.
    assert_eq!(store.enqueue(original.clone()).await, Err(rejected));
    let db = db.lock().unwrap();
    let count: u64 = db
        .query_row("select count(*) from agent_steering_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let gate_count: u64 = db
        .query_row("select count(*) from agent_steering_gates", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(gate_count, 0);
    assert_eq!(original.text, "preserve this queued instruction");
}

#[tokio::test]
async fn wrong_expected_turn_precedes_compact_task_rejection() {
    let (db, stale) = setup_kind(Connection::open_in_memory().unwrap(), TaskKind::Compact);
    let actual_turn_id = Uuid::new_v4();
    append(
        &db,
        stale,
        vec![
            EventPayload::TurnFinished {
                turn_id: stale.turn_id,
                outcome: TurnOutcome::Completed,
            },
            EventPayload::TurnStarted {
                turn_id: actual_turn_id,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "test".into(),
                    revision: None,
                },
                task_kind: TaskKind::Compact,
            },
        ],
    );
    assert_eq!(
        SqliteSteeringStore::new(db)
            .enqueue(input(stale, "instruction"))
            .await,
        Err(SteeringError::ExpectedTurnMismatch { actual_turn_id })
    );
}
