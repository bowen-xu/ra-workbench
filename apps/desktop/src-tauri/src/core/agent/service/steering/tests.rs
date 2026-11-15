use super::*;
use crate::agent::repository::{
    SqliteConversationStore, SqliteSubmissionStore, steering::SqliteSteeringStore,
};
use research_agent_core::{
    protocol::ModelIdentity,
    steering::{SteeringError, SteeringInput},
    submission::{SubmissionInput, SubmissionStore},
};
use rusqlite::Connection;

async fn setup() -> (
    Arc<Mutex<Connection>>,
    AgentSessionService,
    SqliteSteeringStore,
    SteeringScope,
) {
    setup_connection(Connection::open_in_memory().unwrap()).await
}
async fn setup_connection(
    mut db: Connection,
) -> (
    Arc<Mutex<Connection>>,
    AgentSessionService,
    SqliteSteeringStore,
    SteeringScope,
) {
    crate::storage::migrate(&mut db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let service = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "test".into()).unwrap(),
    ));
    let id = ConversationId::from_uuid(Uuid::new_v4());
    service.create(id).await.unwrap();
    let record = SqliteSubmissionStore::new(db.clone())
        .put(
            *id.as_uuid(),
            Uuid::new_v4(),
            SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                text: "initial".into(),
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
    service
        .commit_payload(
            id,
            record.request_id,
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: record.turn_id,
                context_snapshot_id: record.context_snapshot_id,
                model: record.input.model,
            },
        )
        .await
        .unwrap();
    let store = SqliteSteeringStore::new(db.clone());
    (
        db,
        service,
        store,
        SteeringScope {
            conversation_id: *id.as_uuid(),
            turn_id: record.turn_id,
        },
    )
}
fn input(scope: SteeringScope) -> SteeringInput {
    SteeringInput {
        skill_instructions: vec![],
        user_input: vec![],
        scope,
        input_id: Uuid::new_v4(),
        text: "additional evidence".into(),
    }
}

#[tokio::test]
async fn stop_fence_preserves_pending_input_and_rejects_later_arrivals() {
    use research_agent_core::{protocol::TurnOutcome, steering::SteeringRecoveryStore};
    let (_db, service, store, scope) = setup().await;
    let first = input(scope);
    store.enqueue(first.clone()).await.unwrap();
    let source = SessionTurnInput {
        evidence_store: None,
        service: &service,
        store: &store,
        scope,
        injected: std::sync::Mutex::default(),
    };
    assert!(source.seal_for_stop(Uuid::new_v4()).await.is_err());
    source.seal_for_stop(scope.turn_id).await.unwrap();
    source.seal_for_stop(scope.turn_id).await.unwrap();
    assert_eq!(
        store.pending(scope, false).await.unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store.enqueue(input(scope)).await.unwrap_err(),
        SteeringError::Closed
    );
    let id = ConversationId::from_uuid(scope.conversation_id);
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: TurnOutcome::Completed,
            },
        )
        .await
        .unwrap();
    assert_eq!(store.recoverable(scope).await.unwrap(), vec![first]);
    assert!(source.seal_for_stop(scope.turn_id).await.is_err());
}

#[tokio::test]
async fn multiple_structured_steers_inject_once_and_restore_frozen_bodies() {
    use research_agent_core::{
        skills::{SkillIdentity, instructions::SkillInstructions},
        user_input::UserInput,
    };
    let (db, service, store, scope) = setup().await;
    let identity = SkillIdentity {
        environment_id: "application".into(),
        source_id: ".agents/skills/review/SKILL.md".into(),
        revision: "v1".into(),
    };
    let make = |text: &str| SteeringInput {
        scope,
        input_id: Uuid::new_v4(),
        text: text.into(),
        user_input: vec![
            UserInput::Text {
                text: text.into(),
                text_elements: vec![],
            },
            UserInput::Skill {
                name: "review".into(),
                identity: identity.clone(),
            },
        ],
        skill_instructions: vec![SkillInstructions {
            identity: identity.clone(),
            name: "review".into(),
            contents: "Frozen review instructions".into(),
        }],
    };
    let first = make("@review first");
    let second = make("@review second");
    store.enqueue(first.clone()).await.unwrap();
    store.enqueue(second.clone()).await.unwrap();
    let source = SessionTurnInput {
        evidence_store: None,
        service: &service,
        store: &store,
        scope,
        injected: std::sync::Mutex::default(),
    };
    let messages = source.drain(scope.turn_id, false).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert!(
        matches!(&messages[0], ModelMessage::User { content: parts, .. } if parts.len() == 2 && matches!(&parts[0], ContentPart::Text(text) if text.contains("Frozen review instructions")))
    );
    assert!(
        matches!(&messages[1], ModelMessage::User { content: parts, .. }
        if matches!(parts.as_slice(), [ContentPart::Text(text)] if text == &second.text))
    );
    let saved = SqliteSubmissionStore::new(db.clone())
        .get_history_steering(scope.conversation_id, scope.turn_id)
        .await
        .unwrap();
    assert_eq!(saved, vec![first, second]);
    SqliteSteeringStore::discard_unconsumed_on_startup(&mut db.lock().unwrap()).unwrap();
    assert_eq!(
        SqliteSubmissionStore::new(db.clone())
            .get_history_steering(scope.conversation_id, scope.turn_id)
            .await
            .unwrap(),
        saved
    );
    // Forks resolve resources against the original immutable input snapshot,
    // rather than rediscovering a same-name skill from the current catalog.
    use research_agent_core::conversation::{
        BindingKind, ConversationBinding, ConversationForkRequest,
    };
    {
        let db = db.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values('note','Fixture','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','note')", [scope.conversation_id.to_string()]).unwrap();
    }
    service
        .commit_payload(
            ConversationId::from_uuid(scope.conversation_id),
            Uuid::new_v4(),
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: research_agent_core::protocol::TurnOutcome::Completed,
            },
        )
        .await
        .unwrap();
    let child = Uuid::new_v4();
    crate::agent::repository::SqliteConversationCatalog::new(db.clone())
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(child),
            parent_id: ConversationId::from_uuid(scope.conversation_id),
            through_turn_id: scope.turn_id,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note".into(),
            },
        })
        .await
        .unwrap();
    assert_eq!(
        SqliteSubmissionStore::new(db)
            .get_history_steering(child, scope.turn_id)
            .await
            .unwrap(),
        saved
    );
}

#[tokio::test]
async fn concurrent_consumers_commit_input_once_then_seal() {
    let (_, service, store, scope) = setup().await;
    let input = input(scope);
    store.enqueue(input.clone()).await.unwrap();
    let source = SessionTurnInput {
        evidence_store: None,
        injected: std::sync::Mutex::default(),
        service: &service,
        store: &store,
        scope,
    };
    let (first, second) = tokio::join!(
        source.drain(scope.turn_id, false),
        source.drain(scope.turn_id, false)
    );
    assert_eq!(first.unwrap().len() + second.unwrap().len(), 1);
    let history = service
        .open(ConversationId::from_uuid(scope.conversation_id))
        .await
        .unwrap();
    let turn = history.turn(scope.turn_id).unwrap();
    assert_eq!(turn.items.len(), 1);
    assert!(turn.items[&input.input_id].completed);
    assert_eq!(turn.items[&input.input_id].text, input.text);
    assert!(source.drain(scope.turn_id, true).await.unwrap().is_empty());
    assert_eq!(
        store
            .enqueue(SteeringInput {
                input_id: Uuid::new_v4(),
                ..input
            })
            .await,
        Err(SteeringError::Closed)
    );
}

#[tokio::test]
async fn failed_journal_commit_never_returns_model_input() {
    let (db, service, store, scope) = setup().await;
    store.enqueue(input(scope)).await.unwrap();
    db.lock().unwrap().execute_batch("create trigger reject_steering before insert on agent_journal_records begin select raise(abort, 'fixture failure'); end;").unwrap();
    let source = SessionTurnInput {
        evidence_store: None,
        injected: std::sync::Mutex::default(),
        service: &service,
        store: &store,
        scope,
    };
    assert!(matches!(
        source.drain(scope.turn_id, false).await,
        Err(RunError::EventSink)
    ));
    assert_eq!(store.pending(scope, false).await.unwrap().len(), 1);
    assert!(matches!(
        source.drain(scope.turn_id, false).await,
        Err(RunError::EventSink)
    ));
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row("select count(*) from agent_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[path = "restart_tests.rs"]
mod restart;

#[tokio::test]
async fn nonsealing_inspection_allows_later_input_and_checks_turn_identity() {
    let (_db, service, store, scope) = setup().await;
    let source = SessionTurnInput {
        evidence_store: None,
        service: &service,
        store: &store,
        scope,
        injected: std::sync::Mutex::default(),
    };
    assert!(matches!(
        source.has_pending(Uuid::new_v4()).await,
        Err(RunError::EventSink)
    ));
    assert!(!source.has_pending(scope.turn_id).await.unwrap());
    let first = input(scope);
    store.enqueue(first.clone()).await.unwrap();
    assert!(source.has_pending(scope.turn_id).await.unwrap());
    assert_eq!(store.pending(scope, false).await.unwrap(), vec![first]);
    assert_eq!(source.drain(scope.turn_id, false).await.unwrap().len(), 1);
    assert!(!source.has_pending(scope.turn_id).await.unwrap());
    store.enqueue(input(scope)).await.unwrap();
}

#[tokio::test]
async fn pending_inspection_keeps_history_unchanged_and_empty_inspection_seals() {
    let (_db, service, store, scope) = setup().await;
    let first = input(scope);
    store.enqueue(first.clone()).await.unwrap();
    let source = SessionTurnInput {
        evidence_store: None,
        service: &service,
        store: &store,
        scope,
        injected: std::sync::Mutex::default(),
    };
    let id = ConversationId::from_uuid(scope.conversation_id);
    let before = service
        .slot(id, false)
        .unwrap()
        .lock()
        .await
        .as_ref()
        .unwrap()
        .projection()
        .seq();
    assert!(source.has_pending_or_seal(scope.turn_id).await.unwrap());
    assert!(source.has_pending_or_seal(scope.turn_id).await.unwrap());
    assert_eq!(
        service
            .slot(id, false)
            .unwrap()
            .lock()
            .await
            .as_ref()
            .unwrap()
            .projection()
            .seq(),
        before
    );
    let second = input(scope);
    store.enqueue(second).await.unwrap();
    assert_eq!(source.drain(scope.turn_id, false).await.unwrap().len(), 2);
    assert!(!source.has_pending_or_seal(scope.turn_id).await.unwrap());
    assert_eq!(
        store.enqueue(input(scope)).await.unwrap_err(),
        SteeringError::Closed
    );
    // The finishing fence seals admission, not the owned turn. Finalization and
    // its model continuation may still commit while later steering is rejected.
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::PhaseChanged {
                turn_id: scope.turn_id,
                phase: research_agent_core::protocol::TurnPhase::RunningModel,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .slot(id, false)
            .unwrap()
            .lock()
            .await
            .as_ref()
            .unwrap()
            .projection()
            .active_turn(),
        Some(scope.turn_id)
    );
}
