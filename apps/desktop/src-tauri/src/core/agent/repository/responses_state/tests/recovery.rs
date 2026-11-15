use super::inheritance::{encode, item};
use super::*;
use research_agent_core::{
    model::*,
    protocol::{ConversationProjection, ItemKind},
};

pub(super) fn open_turn(
    db: &mut Connection,
    previous: &ResponsesStateScope,
    text: &str,
) -> ResponsesStateScope {
    let next = ResponsesStateScope {
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        ..previous.clone()
    };
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: text.into(),
        model: next.model.clone(),
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values (?1,?2,?3,?4,1,?5)",
        params![next.conversation_id.to_string(),Uuid::new_v4().to_string(),next.turn_id.to_string(),next.context_snapshot_id.to_string(),serde_json::to_string(&input).unwrap()]).unwrap();
    append(
        db,
        &next,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: next.turn_id,
            context_snapshot_id: next.context_snapshot_id,
            model: next.model.clone(),
        },
    );
    next
}
pub(super) fn projection(db: &Connection, scope: &ResponsesStateScope) -> ConversationProjection {
    let mut history = ConversationProjection::new(ConversationId::from_uuid(scope.conversation_id));
    let events: Vec<String> = db
        .prepare("select payload_json from agent_items where conversation_id=?1 order by ordinal")
        .unwrap()
        .query_map([scope.conversation_id.to_string()], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for event in events {
        history
            .apply(&serde_json::from_str(&event).unwrap())
            .unwrap();
    }
    history
}

#[tokio::test]
async fn completed_owned_turn_without_receipt_still_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-receipt.sqlite3");
    let scope = initialize(&path);
    let db = connection(&path);
    {
        let mut db = db.lock().unwrap();
        append(
            &mut db,
            &scope,
            EventPayload::ModelResponseStarted {
                turn_id: scope.turn_id,
                attempt_id: INITIAL_ATTEMPT,
                response_id: "confirmed".into(),
                model: Some(scope.model.clone()),
            },
        );
        append(
            &mut db,
            &scope,
            EventPayload::ModelRequestFinished {
                turn_id: scope.turn_id,
                attempt_id: INITIAL_ATTEMPT,
                input_tokens: Some(1),
                output_tokens: Some(1),
            },
        );
        append(
            &mut db,
            &scope,
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: TurnOutcome::Completed,
            },
        );
    }
    let next = open_turn(&mut db.lock().unwrap(), &scope, "next");
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(1), next.model, input())
            .await,
        Err(ModelError::InvalidRequest)
    ));
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn failures_before_response_allow_explicit_next_turn_without_replaying_requests() {
    for outcome in [
        TurnOutcome::Failed,
        TurnOutcome::Canceled,
        TurnOutcome::Interrupted,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.sqlite3");
        let scope = initialize(&path);
        let db = connection(&path);
        item(
            &mut db.lock().unwrap(),
            &scope,
            ItemKind::UserMessage,
            "read",
        );
        let first = encode(
            &scope,
            vec![ModelMessage::User {
                content: vec![ContentPart::Text("read".into())],
                source_item_id: None,
            }],
        );
        let old = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
        if outcome != TurnOutcome::Failed {
            // Headers wait cancellation has an empty admission checkpoint;
            // authentication failure does not reach prepare_input at all.
            old.fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), first)
                .await
                .unwrap();
        }
        append(
            &mut db.lock().unwrap(),
            &scope,
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome,
            },
        );
        let history = projection(&db.lock().unwrap(), &scope);
        let mut replay = reconstruct_history(&history).unwrap();
        replay.push(ModelMessage::User {
            content: vec![ContentPart::Text("next".into())],
            source_item_id: None,
        });
        let next = open_turn(&mut db.lock().unwrap(), &scope, "next");
        let normalized = encode(&next, replay);
        drop(old);
        drop(db);
        let store = SqliteResponsesState::new(connection(&path), next.clone(), 65536).unwrap();
        assert_eq!(
            store
                .fixture_prepare_input(Uuid::from_u128(1), next.model.clone(), normalized.clone())
                .await
                .unwrap(),
            normalized
        );
        let db = connection(&path);
        assert_eq!(db.lock().unwrap().query_row(
            "select count(*) from agent_items where json_extract(payload_json,'$.payload.type')='modelRequestStarted'", [],
            |r| r.get::<_, u64>(0)).unwrap(), 1);
    }
}

#[tokio::test]
async fn unconfirmed_native_response_requires_reconciliation_but_partial_journal_can_retry() {
    for native_only in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ambiguous.sqlite3");
        let scope = initialize(&path);
        let db = connection(&path);
        item(
            &mut db.lock().unwrap(),
            &scope,
            ItemKind::UserMessage,
            "read",
        );
        let input = encode(
            &scope,
            vec![ModelMessage::User {
                content: vec![ContentPart::Text("read".into())],
                source_item_id: None,
            }],
        );
        let old = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
        old.fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input.clone())
            .await
            .unwrap();
        if native_only {
            let mut received = receipt(&scope);
            received.normalized_input = input.clone();
            received.input = input;
            old.fixture_preserve(received).await.unwrap();
        } else {
            append(
                &mut db.lock().unwrap(),
                &scope,
                EventPayload::ModelResponseStarted {
                    turn_id: scope.turn_id,
                    attempt_id: INITIAL_ATTEMPT,
                    response_id: "partial".into(),
                    model: Some(scope.model.clone()),
                },
            );
            item(
                &mut db.lock().unwrap(),
                &scope,
                ItemKind::AssistantMessage,
                "partial response",
            );
        }
        append(
            &mut db.lock().unwrap(),
            &scope,
            EventPayload::TurnFinished {
                turn_id: scope.turn_id,
                outcome: TurnOutcome::Interrupted,
            },
        );
        let mut replay = reconstruct_history(&projection(&db.lock().unwrap(), &scope)).unwrap();
        replay.push(ModelMessage::User {
            content: vec![ContentPart::Text("next".into())],
            source_item_id: None,
        });
        let next = open_turn(&mut db.lock().unwrap(), &scope, "next");
        let normalized = encode(&next, replay);
        let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
        let result = store
            .fixture_prepare_input(Uuid::from_u128(1), next.model, normalized)
            .await;
        assert!(
            result.is_ok(),
            "an interrupted turn must allow a fresh retry without opaque replay"
        );
        let checkpoints = db
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints where turn_id=?1",
                [next.turn_id.to_string()],
                |r| r.get::<_, u64>(0),
            )
            .unwrap();
        assert_eq!(checkpoints, 1);
    }
}
