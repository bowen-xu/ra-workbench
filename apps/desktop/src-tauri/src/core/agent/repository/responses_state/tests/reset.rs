use super::*;
use research_agent_core::context::{
    checkpoint::{CanonicalContext, ContextCheckpoint},
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase},
};

pub(super) fn install_reset(db: &mut Connection, scope: &ResponsesStateScope) {
    let checkpoint = ContextCheckpoint {
        schema_version: 2,
        window_id: Uuid::now_v7(),
        conversation_id: scope.conversation_id,
        turn_id: scope.turn_id,
        source_watermark: 3,
        previous_window_id: None,
        binding: CompactionBinding {
            model: scope.model.clone(),
            strategy_id: "token-budget.reset-window".into(),
            state_schema: 1,
            kind: CompactionKind::ResetWindow,
        },
        credential_scope_id: None,
        phase: CompactionPhase::MidTurn,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: vec![],
        }),
    };
    crate::agent::repository::context_windows::install_owned(db, &checkpoint, 3, None).unwrap();
}

#[tokio::test]
async fn reset_fences_old_receipts_and_preserves_new_epoch_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize_through(&path, 127);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
        .await
        .unwrap();
    store.fixture_preserve(receipt(&scope)).await.unwrap();
    install_reset(&mut db.lock().unwrap(), &scope);
    assert!(matches!(
        store.fixture_preserve(receipt(&scope)).await,
        Err(ModelError::InvalidRequest)
    ));
    let fresh = vec![json!({"role":"user","content":"new window"})];
    assert_eq!(
        store
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), fresh.clone())
            .await
            .unwrap(),
        fresh
    );
    drop(store);
    let store = SqliteResponsesState::new(connection(&path), scope.clone(), 100_000).unwrap();
    assert!(matches!(
        store.fixture_preserve(receipt(&scope)).await,
        Err(ModelError::InvalidRequest)
    ));
    let attempt = Uuid::new_v4();
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelRequestFinished {
            turn_id: scope.turn_id,
            attempt_id: INITIAL_ATTEMPT,
            input_tokens: None,
            output_tokens: None,
        },
    );
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelRequestStarted {
            turn_id: scope.turn_id,
            attempt_id: attempt,
            model: scope.model.clone(),
        },
    );
    let mut received = receipt(&scope);
    received.request_id = attempt;
    received.input = fresh.clone();
    received.normalized_input = fresh.clone();
    store.fixture_preserve(received).await.unwrap();
    let mut replay = receipt(&scope);
    replay.request_id = attempt;
    replay.input = fresh.clone();
    replay.normalized_input = fresh.clone();
    store.fixture_preserve(replay).await.unwrap();
    drop(store);
    let store = SqliteResponsesState::new(connection(&path), scope.clone(), 100_000).unwrap();
    let mut next = fresh;
    next.push(next_input()[1].clone());
    next.push(json!({"role":"user","content":"continue new window"}));
    let wire = store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), next.clone())
        .await
        .unwrap();
    assert_eq!(wire[0]["content"], "new window");
    assert_eq!(wire[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(wire.len(), 4);
    assert!(!wire.iter().any(|item| item["content"] == "read"));
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(2), scope.model, next)
            .await,
        Err(ModelError::Authentication)
    ));
}
