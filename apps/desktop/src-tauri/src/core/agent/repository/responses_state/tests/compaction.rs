use super::*;
use research_agent_core::context::{
    checkpoint::{CanonicalContext, ContextCheckpoint, StoredContent, StoredMessage},
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase},
};

fn install(db: &mut Connection, scope: &ResponsesStateScope, output: CompactionOutput) {
    install_with_retained(db, scope, output, vec![], None);
}
fn install_with_retained(
    db: &mut Connection,
    scope: &ResponsesStateScope,
    output: CompactionOutput,
    retained: Vec<StoredMessage>,
    canonical_context: Option<CanonicalContext>,
) {
    let kind = match output {
        CompactionOutput::ResetWindow => CompactionKind::ResetWindow,
        CompactionOutput::RemoteOpaque(_) => CompactionKind::RemoteOpaque,
        CompactionOutput::RemoteWindow(_) => CompactionKind::RemoteWindow,
        _ => unreachable!(),
    };
    let checkpoint = ContextCheckpoint {
        schema_version: if canonical_context.is_some() { 2 } else { 1 },
        window_id: Uuid::new_v4(),
        conversation_id: scope.conversation_id,
        turn_id: scope.turn_id,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model: scope.model.clone(),
            strategy_id: "fixture.responses.v2".into(),
            state_schema: 1,
            kind,
        },
        credential_scope_id: Some(Uuid::from_u128(1)),
        phase: if canonical_context.is_some() {
            CompactionPhase::MidTurn
        } else {
            CompactionPhase::PreTurn
        },
        output,
        retained,
        canonical_context,
    };
    crate::agent::repository::context_windows::install_owned(db, &checkpoint, 3, None).unwrap();
}
fn opaque() -> Value {
    json!({"type":"compaction","encrypted_content":"installed-opaque","vendor_extra":{"v":2}})
}
fn replacement() -> Vec<Value> {
    vec![
        json!({"role":"developer","content":"current authority"}),
        json!({"role":"user","content":"retained and fresh"}),
    ]
}
fn saved(db: &Connection) -> String {
    db.query_row(
        "select checkpoint_json from agent_responses_checkpoints",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn retained_users_precede_opaque_and_fresh_input_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    install_with_retained(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteOpaque(opaque()),
        vec![StoredMessage::User {
            source_item_id: None,
            content: vec![StoredContent::Text {
                text: "retained".into(),
            }],
        }],
        None,
    );
    let normalized = vec![
        replacement()[0].clone(),
        json!({"role":"user","content":"retained"}),
        json!({"role":"user","content":"fresh"}),
    ];
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let wire = state
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), normalized.clone())
        .await
        .unwrap();
    assert_eq!(
        wire,
        vec![
            normalized[0].clone(),
            normalized[1].clone(),
            opaque(),
            normalized[2].clone()
        ]
    );
    let mut received = receipt(&scope);
    received.normalized_input = normalized.clone();
    received.input = wire.clone();
    state.fixture_preserve(received).await.unwrap();
    drop(state);
    drop(db);
    let reopened = SqliteResponsesState::new(connection(&path), scope.clone(), 100_000).unwrap();
    let mut next = normalized;
    next.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}));
    next.push(json!({"role":"user","content":"continue"}));
    let continued = reopened
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), next)
        .await
        .unwrap();
    assert_eq!(&continued[..wire.len()], wire.as_slice());
    assert_eq!(
        continued
            .iter()
            .filter(|item| item["type"] == "compaction")
            .count(),
        1
    );
}

#[tokio::test]
async fn v2_cannot_replay_retained_assistant_messages() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    install_with_retained(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteOpaque(opaque()),
        vec![StoredMessage::Assistant {
            phase: None,
            content: vec![StoredContent::Text {
                text: "assistant reply".into(),
            }],
            tool_calls: vec![],
        }],
        None,
    );
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    assert!(matches!(
        state
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), replacement())
            .await,
        Err(ModelError::UnsupportedCapability)
    ));
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn canonical_mid_turn_context_remains_before_last_retained_user_and_opaque() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize_through(&path, 127);
    let db = connection(&path);
    let user = |text: &str| StoredMessage::User {
        source_item_id: None,
        content: vec![StoredContent::Text { text: text.into() }],
    };
    install_with_retained(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteOpaque(opaque()),
        vec![user("earlier"), user("last")],
        Some(CanonicalContext {
            before_retained: 1,
            messages: vec![user("canonical")],
        }),
    );
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let normalized = vec![
        replacement()[0].clone(),
        json!({"role":"user","content":"earlier"}),
        json!({"role":"user","content":"canonical"}),
        json!({"role":"user","content":"last"}),
        json!({"role":"user","content":"fresh"}),
    ];
    let mut expected = normalized.clone();
    expected.insert(4, opaque());
    assert_eq!(
        state
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), normalized)
            .await
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn installed_opaque_survives_reopen_and_continuation_without_normalized_text() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    let account = Uuid::from_u128(1);
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    state
        .fixture_prepare_input(account, scope.model.clone(), input())
        .await
        .unwrap();
    state.fixture_preserve(receipt(&scope)).await.unwrap();
    install(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteOpaque(opaque()),
    );
    let normalized = replacement();
    let wire = state
        .fixture_prepare_input(account, scope.model.clone(), normalized.clone())
        .await
        .unwrap();
    assert_eq!(
        wire,
        vec![normalized[0].clone(), opaque(), normalized[1].clone()]
    );
    assert!(
        !serde_json::to_string(&wire)
            .unwrap()
            .contains("opaque-fixture")
    );
    let mut received = receipt(&scope);
    received.normalized_input = normalized.clone();
    received.input = wire.clone();
    state.fixture_preserve(received).await.unwrap();
    let persisted: Value = serde_json::from_str(&saved(&db.lock().unwrap())).unwrap();
    assert!(
        !persisted["checkpoint"]["normalized_prefix"]
            .to_string()
            .contains("installed-opaque")
    );
    assert_eq!(persisted["checkpoint"]["wire_prefix"][1], opaque());
    drop(state);
    drop(db);
    let db = connection(&path);
    let reopened = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let mut next = normalized;
    next.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}));
    next.push(json!({"role":"user","content":"continue"}));
    let continued = reopened
        .fixture_prepare_input(account, scope.model.clone(), next.clone())
        .await
        .unwrap();
    assert_eq!(
        continued
            .iter()
            .filter(|item| item["type"] == "compaction")
            .count(),
        1
    );
    assert_eq!(continued[1], opaque());
    let before = saved(&db.lock().unwrap());
    assert!(matches!(
        reopened
            .fixture_prepare_input(Uuid::from_u128(2), scope.model.clone(), next)
            .await,
        Err(ModelError::Authentication)
    ));
    assert_eq!(saved(&db.lock().unwrap()), before);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row("select durable_count from agent_conversations", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn unsupported_or_unbound_installed_state_cannot_become_ordinary_input() {
    for output in [CompactionOutput::RemoteOpaque(
        json!({"type":"vendor_memory","state":"foreign"}),
    )] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.sqlite");
        let scope = initialize(&path);
        let db = connection(&path);
        install(&mut db.lock().unwrap(), &scope, output);
        let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
        assert!(matches!(
            state
                .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), replacement())
                .await,
            Err(ModelError::UnsupportedCapability)
        ));
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row(
                    "select count(*) from agent_responses_checkpoints",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn full_window_survives_reopen_without_projection_or_duplicate_items() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    let items = vec![
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"retained"}]}),
        opaque(),
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"older answer"}]}),
        json!({"type":"future_provider_item","payload":{"version":3}}),
    ];
    install(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteWindow(items.clone()),
    );
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let normalized = replacement();
    let wire = state
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), normalized.clone())
        .await
        .unwrap();
    let mut expected = vec![normalized[0].clone()];
    expected.extend(items.clone());
    expected.push(normalized[1].clone());
    assert_eq!(wire, expected);
    let before = saved(&db.lock().unwrap());
    let mut forged = receipt(&scope);
    forged.normalized_input = normalized.clone();
    forged.input = normalized.clone();
    assert_eq!(
        state.fixture_preserve(forged).await,
        Err(ModelError::InvalidRequest)
    );
    assert_eq!(saved(&db.lock().unwrap()), before);
    let mut received = receipt(&scope);
    received.normalized_input = normalized.clone();
    received.input = wire.clone();
    let generated = received.output.clone();
    state.fixture_preserve(received).await.unwrap();
    drop(state);
    drop(db);
    let db = connection(&path);
    let reopened = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    let mut next = normalized;
    next.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}));
    next.push(json!({"role":"user","content":"continue"}));
    let continued = reopened
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), next.clone())
        .await
        .unwrap();
    assert_eq!(&continued[1..1 + items.len()], items.as_slice());
    let mut expected = wire;
    expected.extend(generated);
    expected.push(json!({"role":"user","content":"continue"}));
    assert_eq!(continued, expected);
    let before = saved(&db.lock().unwrap());
    assert_eq!(
        reopened
            .fixture_prepare_input(Uuid::from_u128(2), scope.model.clone(), next)
            .await
            .err(),
        Some(ModelError::Authentication)
    );
    assert_eq!(saved(&db.lock().unwrap()), before);
}

#[tokio::test]
async fn forged_completion_without_installed_item_rolls_back_the_replay_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let scope = initialize(&path);
    let db = connection(&path);
    install(
        &mut db.lock().unwrap(),
        &scope,
        CompactionOutput::RemoteOpaque(opaque()),
    );
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
    state
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), replacement())
        .await
        .unwrap();
    let before = saved(&db.lock().unwrap());
    let mut received = receipt(&scope);
    received.normalized_input = replacement();
    received.input = replacement();
    assert!(matches!(
        state.fixture_preserve(received).await,
        Err(ModelError::InvalidRequest)
    ));
    assert_eq!(saved(&db.lock().unwrap()), before);
    let mut alias = receipt(&scope);
    alias.normalized_input = replacement();
    alias.input = vec![
        alias.normalized_input[0].clone(),
        opaque(),
        alias.normalized_input[1].clone(),
    ];
    alias.model.model_id = "different-model".into();
    assert!(matches!(
        state.fixture_preserve(alias).await,
        Err(ModelError::UnsupportedCapability)
    ));
    assert_eq!(saved(&db.lock().unwrap()), before);
}

#[tokio::test]
async fn native_cross_model_read_preserves_producer_and_requires_frozen_codex_auth() {
    use crate::agent::{
        execution_profile::NativeCompactionPolicy,
        repository::{
            SqliteSubmissionStore,
            context_policy::{ContextAuthentication, FrozenContextPolicy, SqliteContextPolicy},
        },
    };
    use research_agent_core::{
        context::accounting::AutoCompactionScope, submission::SubmissionStore,
    };
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        for source_auth in [
            ContextAuthentication::CodexBackend,
            ContextAuthentication::ApiKey,
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("read.sqlite");
            let old = initialize_model(
                &path,
                130,
                ModelIdentity {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    revision: Some("old".into()),
                },
            );
            let db = connection(&path);
            let submissions = SqliteSubmissionStore::new(db.clone());
            let request: String = db
                .lock()
                .unwrap()
                .query_row(
                    "select request_id from agent_submissions where turn_id=?1",
                    [old.turn_id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            let record = submissions
                .get(old.conversation_id, Uuid::parse_str(&request).unwrap())
                .await
                .unwrap();
            let source = FrozenContextPolicy {
                tool_truncation_policy: None,
                reasoning_summary: None,
                chat_developer_model: None,
                compact_prompt: None,
                token_budget_settings: None,
                schema_version: 1,
                binding: native.binding(old.model.clone()),
                budget: None,
                max_output_tokens: 128,
                scope: AutoCompactionScope::Total,
                unknown_window_trigger_tokens: None,
                comp_hash: None,
                authentication: source_auth,
            };
            SqliteContextPolicy::new(db.clone(), &record, source.clone())
                .unwrap()
                .prepare()
                .await
                .unwrap();
            let output = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
                CompactionOutput::RemoteOpaque(opaque())
            } else {
                CompactionOutput::RemoteWindow(vec![
                    json!({"role":"user","content":"supplier-retained"}),
                    opaque(),
                ])
            };
            let checkpoint = ContextCheckpoint {
                schema_version: 1,
                window_id: Uuid::new_v4(),
                conversation_id: old.conversation_id,
                turn_id: old.turn_id,
                source_watermark: 1,
                previous_window_id: None,
                binding: source.binding.clone(),
                credential_scope_id: Some(Uuid::from_u128(1)),
                phase: CompactionPhase::PreTurn,
                output,
                retained: vec![],
                canonical_context: None,
            };
            crate::agent::repository::context_windows::install_owned(
                &mut db.lock().unwrap(),
                &checkpoint,
                3,
                None,
            )
            .unwrap();
            let mut next_input = record.input;
            next_input.model.model_id = "gpt-5.6-sol".into();
            next_input.model.revision = Some("new".into());
            let next = submissions
                .put(old.conversation_id, Uuid::new_v4(), next_input)
                .await
                .unwrap();
            let scope = ResponsesStateScope {
                conversation_id: old.conversation_id,
                turn_id: next.turn_id,
                context_snapshot_id: next.context_snapshot_id,
                model: next.input.model.clone(),
            };
            {
                let mut connection = db.lock().unwrap();
                append(
                    &mut connection,
                    &old,
                    EventPayload::ModelRequestFailed {
                        turn_id: old.turn_id,
                        attempt_id: INITIAL_ATTEMPT,
                        error: "synthetic closed attempt".into(),
                        retryable: false,
                        category: None,
                    },
                );
                append(
                    &mut connection,
                    &old,
                    EventPayload::TurnFinished {
                        turn_id: old.turn_id,
                        outcome: TurnOutcome::Canceled,
                    },
                );
                append(
                    &mut connection,
                    &scope,
                    EventPayload::TurnStarted {
                        turn_id: scope.turn_id,
                        context_snapshot_id: scope.context_snapshot_id,
                        model: scope.model.clone(),
                        task_kind: research_agent_core::protocol::TaskKind::Regular,
                    },
                );
            }
            let exact = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000).unwrap();
            assert!(matches!(
                exact
                    .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), replacement())
                    .await,
                Err(ModelError::UnsupportedCapability)
            ));
            let native_state = SqliteResponsesState::new(db.clone(), scope.clone(), 100_000)
                .unwrap()
                .with_native_read_policy(native);
            let read = native_state
                .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), replacement())
                .await;
            if source_auth == ContextAuthentication::CodexBackend {
                let wire = read.unwrap();
                assert!(wire.contains(&opaque()));
                if native == NativeCompactionPolicy::ChatGptResponsesFullWindow {
                    assert!(
                        wire.iter()
                            .any(|item| item["content"] == "supplier-retained")
                    );
                }
                assert!(matches!(
                    native_state
                        .fixture_prepare_input(
                            Uuid::from_u128(2),
                            scope.model.clone(),
                            replacement()
                        )
                        .await,
                    Err(ModelError::Authentication)
                ));
                drop(native_state);
                let reopened = SqliteResponsesState::new(connection(&path), scope.clone(), 100_000)
                    .unwrap()
                    .with_native_read_policy(native);
                assert_eq!(
                    reopened
                        .fixture_prepare_input(
                            Uuid::from_u128(1),
                            scope.model.clone(),
                            replacement()
                        )
                        .await
                        .unwrap(),
                    wire
                );
                db.lock()
                    .unwrap()
                    .execute(
                        "delete from agent_context_policies where turn_id=?1",
                        [old.turn_id.to_string()],
                    )
                    .unwrap();
                assert!(matches!(
                    reopened
                        .fixture_prepare_input(
                            Uuid::from_u128(1),
                            scope.model.clone(),
                            replacement()
                        )
                        .await,
                    Err(ModelError::UnsupportedCapability)
                ));
            } else {
                assert!(matches!(read, Err(ModelError::UnsupportedCapability)));
            }
            assert!(
                crate::agent::repository::context_windows::read_active(
                    &db.lock().unwrap(),
                    old.conversation_id
                )
                .unwrap()
                .unwrap()
                    == checkpoint
            );
        }
    }
}
