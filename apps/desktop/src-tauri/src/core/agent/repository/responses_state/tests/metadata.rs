use super::*;
use crate::agent::model::responses_metadata::ResponsesInputKind;

#[tokio::test]
async fn request_metadata_tracks_installed_chain_and_reopens_with_prepared_input() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("metadata.sqlite");
    let scope = initialize_through(&path, 137);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let prepared = store
        .fixture_prepare_with_metadata(
            Uuid::from_u128(1),
            scope.model.clone(),
            input(),
            ResponsesInputKind::Sampling,
        )
        .await
        .unwrap();
    assert_eq!(prepared.input, input());
    assert_eq!(
        prepared.metadata.unwrap().window_id(),
        format!("{}:0", scope.conversation_id)
    );
    reset::install_reset(&mut db.lock().unwrap(), &scope);
    let fresh = vec![json!({"role":"user","content":"new window"})];
    let prepared = store
        .fixture_prepare_with_metadata(
            Uuid::from_u128(1),
            scope.model.clone(),
            fresh.clone(),
            ResponsesInputKind::Sampling,
        )
        .await
        .unwrap();
    assert_eq!(prepared.input, fresh);
    assert_eq!(
        prepared.metadata.unwrap().window_id(),
        format!("{}:1", scope.conversation_id)
    );
    drop(store);
    drop(db);
    let store = SqliteResponsesState::new(connection(&path), scope.clone(), 100_000).unwrap();
    let prepared = store
        .fixture_prepare_with_metadata(
            Uuid::from_u128(1),
            scope.model.clone(),
            fresh.clone(),
            ResponsesInputKind::TextCompaction,
        )
        .await
        .unwrap();
    assert_eq!(prepared.input, fresh);
    assert_eq!(
        prepared.metadata.unwrap().window_id(),
        format!("{}:1", scope.conversation_id)
    );
}

#[tokio::test]
async fn metadata_preparation_failure_rolls_back_initial_identity_and_input_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("failed.sqlite");
    let scope = initialize_through(&path, 137);
    let db = connection(&path);
    db.lock().unwrap().execute_batch("create trigger fail_prepared_metadata before insert on agent_responses_checkpoints begin select raise(abort,'fixture write failure'); end;").unwrap();
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    assert!(matches!(
        store
            .fixture_prepare_with_metadata(
                Uuid::from_u128(1),
                scope.model.clone(),
                input(),
                ResponsesInputKind::Sampling
            )
            .await,
        Err(ModelError::Unavailable)
    ));
    for table in [
        "agent_initial_context_windows",
        "agent_responses_checkpoints",
    ] {
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row(&format!("select count(*) from {table}"), [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_prepared_metadata")
        .unwrap();
    assert_eq!(
        store
            .fixture_prepare_with_metadata(
                Uuid::from_u128(1),
                scope.model.clone(),
                input(),
                ResponsesInputKind::Sampling
            )
            .await
            .unwrap()
            .metadata
            .unwrap()
            .window_id(),
        format!("{}:0", scope.conversation_id)
    );
}
