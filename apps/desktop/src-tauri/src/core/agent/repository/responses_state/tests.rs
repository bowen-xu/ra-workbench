use super::*;
use research_agent_core::submission::SubmissionInput;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TurnOutcome, TurnPhase},
};
use serde_json::json;
const INITIAL_ATTEMPT: Uuid = Uuid::from_u128(100);

fn append(db: &mut Connection, scope: &ResponsesStateScope, payload: EventPayload) {
    let id = ConversationId::from_uuid(scope.conversation_id);
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [scope.conversation_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let event = AgentEvent::new(id, count + 1, 1, Uuid::new_v4(), payload).unwrap();
    crate::agent::repository::append_batch(db, id, Uuid::new_v4(), count, &[event]).unwrap();
}
fn initialize(path: &std::path::Path) -> ResponsesStateScope {
    initialize_through(path, 125)
}
fn initialize_through(path: &std::path::Path, version: i64) -> ResponsesStateScope {
    initialize_model(
        path,
        version,
        ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
    )
}
fn initialize_model(
    path: &std::path::Path,
    version: i64,
    model: ModelIdentity,
) -> ResponsesStateScope {
    let mut db = Connection::open(path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, version);
    let scope = ResponsesStateScope {
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        model,
    };
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "read".into(),
        model: scope.model.clone(),
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    db.execute(
        "insert into agent_conversations(id,title) values (?1,'fixture')",
        [scope.conversation_id.to_string()],
    )
    .unwrap();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values (?1,?2,?3,?4,1,?5)",
        params![scope.conversation_id.to_string(),Uuid::new_v4().to_string(),scope.turn_id.to_string(),scope.context_snapshot_id.to_string(),serde_json::to_string(&input).unwrap()]).unwrap();
    append(
        &mut db,
        &scope,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: scope.turn_id,
            context_snapshot_id: scope.context_snapshot_id,
            model: scope.model.clone(),
        },
    );
    append(
        &mut db,
        &scope,
        EventPayload::PhaseChanged {
            turn_id: scope.turn_id,
            phase: TurnPhase::RunningModel,
        },
    );
    append(
        &mut db,
        &scope,
        EventPayload::ModelRequestStarted {
            turn_id: scope.turn_id,
            attempt_id: INITIAL_ATTEMPT,
            model: scope.model.clone(),
        },
    );
    scope
}

#[path = "tests/authority.rs"]
mod authority;
#[path = "tests/canonical.rs"]
mod canonical;
#[path = "tests/compaction.rs"]
mod compaction;
#[path = "tests/inheritance.rs"]
mod inheritance;
#[path = "tests/metadata.rs"]
mod metadata;
#[path = "tests/recovery.rs"]
mod recovery;
#[path = "tests/reset.rs"]
mod reset;

#[test]
fn version_two_migration_preserves_existing_checkpoint_and_foreign_key() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 70);
    db.execute(
        "insert into agent_conversations(id,title) values ('c','fixture')",
        [],
    )
    .unwrap();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values ('c','r','t','s',1,'{}')", []).unwrap();
    db.execute("insert into agent_responses_checkpoints(turn_id,schema_version,checkpoint_json,updated_at) values ('t',1,'{\"schema_version\":1}','original')", []).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch(include_str!(
        "../../../../../migrations/0071_agent_checkpoint_inheritance.sql"
    ))
    .unwrap();
    tx.commit().unwrap();
    let (version, data, updated): (u32, String, String) = db
        .query_row(
            "select schema_version,checkpoint_json,updated_at from agent_responses_checkpoints",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (version, data.as_str(), updated.as_str()),
        (1, "{\"schema_version\":1}", "original")
    );
    db.execute(
        "update agent_responses_checkpoints set schema_version=2",
        [],
    )
    .unwrap();
    db.execute("delete from agent_submissions where turn_id='t'", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_responses_checkpoints",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
fn connection(path: &std::path::Path) -> Arc<Mutex<Connection>> {
    let db = Connection::open(path).unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    Arc::new(Mutex::new(db))
}
fn input() -> Vec<Value> {
    vec![json!({"role":"user","content":"read"})]
}
fn output() -> Vec<Value> {
    vec![
        json!({"type":"reasoning","id":"reason","summary":[],"encrypted_content":"opaque-fixture"}),
        json!({"type":"message","id":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}),
    ]
}
fn next_input() -> Vec<Value> {
    let mut value = input();
    value.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}));
    value.push(json!({"role":"user","content":"continue"}));
    value
}
fn receipt(scope: &ResponsesStateScope) -> ResponsesReceipt {
    ResponsesReceipt {
        request_id: INITIAL_ATTEMPT,
        credential_scope_id: Uuid::from_u128(1),
        requested_model: scope.model.clone(),
        model: scope.model.clone(),
        normalized_input: input(),
        authority_items: None,
        input: input(),
        output: output(),
    }
}

#[tokio::test]
async fn closes_reopens_and_restores_wire_prefix_without_running_a_model() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let scope = initialize(&path);
    {
        let store = SqliteResponsesState::new(connection(&path), scope.clone(), 16384).unwrap();
        assert_eq!(
            store
                .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
                .await
                .unwrap(),
            input()
        );
        store.fixture_preserve(receipt(&scope)).await.unwrap();
    }
    let store = SqliteResponsesState::new(connection(&path), scope.clone(), 16384).unwrap();
    let expanded = store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), next_input())
        .await
        .unwrap();
    assert_eq!(expanded[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(expanded.len(), 4);
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(2), scope.model, next_input())
            .await,
        Err(ModelError::Authentication)
    ));
}

#[tokio::test]
async fn validates_submission_scope_and_preserves_checkpoint_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let scope = initialize(&path);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 16384).unwrap();
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
        .await
        .unwrap();
    store.fixture_preserve(receipt(&scope)).await.unwrap();
    let before: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut other = scope.clone();
    other.context_snapshot_id = Uuid::new_v4();
    let wrong = SqliteResponsesState::new(db.clone(), other, 16384).unwrap();
    assert!(
        wrong
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), next_input())
            .await
            .is_err()
    );
    let mut unregistered = receipt(&scope);
    unregistered.request_id = Uuid::new_v4();
    assert!(store.fixture_preserve(unregistered).await.is_err());
    let after: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversations where id=?1",
            [scope.conversation_id.to_string()],
        )
        .unwrap();
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "select count(*) from agent_responses_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn refuses_future_checkpoint_and_archived_owner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let scope = initialize(&path);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 16384).unwrap();
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
        .await
        .unwrap();
    db.lock().unwrap().execute("update agent_responses_checkpoints set checkpoint_json=json_set(checkpoint_json,'$.schema_version',99)",[]).unwrap();
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
            .await,
        Err(ModelError::InvalidStream)
    ));
    db.lock()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(1), scope.model, input())
            .await,
        Err(ModelError::InvalidRequest)
    ));
}

#[tokio::test]
async fn ambiguous_save_can_repeat_same_receipt_but_not_changed_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let scope = initialize(&path);
    let store = SqliteResponsesState::new(connection(&path), scope.clone(), 16384).unwrap();
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
        .await
        .unwrap();
    let first = receipt(&scope);
    let id = first.request_id;
    store.fixture_preserve(first).await.unwrap();
    drop(store);
    append(
        &mut connection(&path).lock().unwrap(),
        &scope,
        EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    let reopened = SqliteResponsesState::new(connection(&path), scope.clone(), 16384).unwrap();
    let mut same = receipt(&scope);
    same.request_id = id;
    reopened.fixture_preserve(same).await.unwrap();
    let mut changed = receipt(&scope);
    let mut invalid = receipt(&scope);
    invalid.request_id = Uuid::nil();
    assert!(matches!(
        reopened.fixture_preserve(invalid).await,
        Err(ModelError::InvalidStream)
    ));
    changed.request_id = id;
    changed.output[0]["encrypted_content"] = json!("changed");
    assert!(matches!(
        reopened.fixture_preserve(changed).await,
        Err(ModelError::InvalidRequest)
    ));
}

#[tokio::test]
async fn terminal_or_superseded_attempt_cannot_publish_a_late_checkpoint() {
    for boundary in ["canceled", "finished", "superseded"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let scope = initialize(&path);
        let db = connection(&path);
        let store = SqliteResponsesState::new(db.clone(), scope.clone(), 16384).unwrap();
        store
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
            .await
            .unwrap();
        let before: String = db
            .lock()
            .unwrap()
            .query_row(
                "select checkpoint_json from agent_responses_checkpoints",
                [],
                |row| row.get(0),
            )
            .unwrap();
        append(
            &mut db.lock().unwrap(),
            &scope,
            if boundary == "canceled" {
                EventPayload::TurnFinished {
                    turn_id: scope.turn_id,
                    outcome: TurnOutcome::Canceled,
                }
            } else {
                EventPayload::ModelRequestFinished {
                    turn_id: scope.turn_id,
                    attempt_id: INITIAL_ATTEMPT,
                    input_tokens: None,
                    output_tokens: None,
                }
            },
        );
        if boundary == "superseded" {
            append(
                &mut db.lock().unwrap(),
                &scope,
                EventPayload::ModelRequestStarted {
                    turn_id: scope.turn_id,
                    attempt_id: Uuid::new_v4(),
                    model: scope.model.clone(),
                },
            );
        }
        assert!(matches!(
            store.fixture_preserve(receipt(&scope)).await,
            Err(ModelError::InvalidRequest)
        ));
        let after: String = db
            .lock()
            .unwrap()
            .query_row(
                "select checkpoint_json from agent_responses_checkpoints",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
    }
}

#[tokio::test]
async fn installed_text_window_resets_the_old_opaque_wire_prefix() {
    use research_agent_core::context::{
        checkpoint::ContextCheckpoint,
        compaction_strategy::{
            CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase,
        },
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let account = Uuid::from_u128(1);
    state
        .fixture_prepare_input(account, scope.model.clone(), input())
        .await
        .unwrap();
    state.fixture_preserve(receipt(&scope)).await.unwrap();
    {
        let mut db = db.lock().unwrap();
        let checkpoint = ContextCheckpoint {
            schema_version: 1,
            window_id: Uuid::new_v4(),
            conversation_id: scope.conversation_id,
            turn_id: scope.turn_id,
            source_watermark: 1,
            previous_window_id: None,
            binding: CompactionBinding {
                model: scope.model.clone(),
                strategy_id: "research.text.v1".into(),
                state_schema: 1,
                kind: CompactionKind::TextSummary,
            },
            credential_scope_id: None,
            phase: CompactionPhase::PreTurn,
            output: CompactionOutput::TextSummary("new installed summary".into()),
            retained: vec![],
            canonical_context: None,
        };
        crate::agent::repository::context_windows::install_owned(&mut db, &checkpoint, 3, None)
            .unwrap();
    }
    let replacement = vec![
        json!({"role":"user", "content":"new installed summary"}),
        json!({"role":"user", "content":"fresh input"}),
    ];
    let wire = state
        .fixture_prepare_input(account, scope.model.clone(), replacement.clone())
        .await
        .unwrap();
    assert_eq!(wire, replacement);
    assert!(
        !serde_json::to_string(&wire)
            .unwrap()
            .contains("opaque-fixture")
    );
    let db = db.lock().unwrap();
    let binding: String = db
        .query_row(
            "select window_id from agent_responses_window_bindings where turn_id=?1",
            [scope.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let active: String = db
        .query_row(
            "select window_id from agent_active_context_windows where conversation_id=?1",
            [scope.conversation_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(binding, active);
}
