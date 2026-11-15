use super::*;
use research_agent_core::conversation::{CatalogMutationFuture, ConversationSummary};
use std::sync::Mutex;

fn saved() -> ConversationFork {
    let timestamp = "2026-09-10T00:00:00.000Z".to_owned();
    ConversationFork {
        conversation: ConversationSummary {
            internal: false,
            id: ConversationId::from_uuid(Uuid::from_u128(2)),
            title: "研究对话 · 分支".into(),
            archived: false,
            durable_count: 8,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        },
        parent_id: ConversationId::from_uuid(Uuid::from_u128(1)),
        through_turn_id: Uuid::from_u128(3),
        parent_end_seq: 5,
    }
}
fn request() -> serde_json::Value {
    serde_json::json!({"childId":Uuid::from_u128(2),"parentId":Uuid::from_u128(1),"throughTurnId":Uuid::from_u128(3),"binding":{"kind":"note","resourceId":"note"}})
}
struct Store {
    calls: Mutex<Vec<ConversationForkRequest>>,
    result: ConversationFork,
}
impl ConversationForkStore for Store {
    fn fork(
        &self,
        request: ConversationForkRequest,
    ) -> CatalogMutationFuture<'_, ConversationFork> {
        self.calls.lock().unwrap().push(request);
        Box::pin(async { Ok(self.result.clone()) })
    }
    fn fork_status(
        &self,
        request: ConversationForkRequest,
    ) -> CatalogMutationFuture<'_, research_agent_core::conversation::CatalogMutationStatus> {
        Box::pin(async move {
            Ok(
                research_agent_core::conversation::CatalogMutationStatus::Committed(
                    research_agent_core::conversation::CatalogMutationProof {
                        operation_id: *request.child_id.as_uuid(),
                        conversation_id: request.child_id,
                        epoch: Uuid::from_u128(99),
                        sql_fence: 7,
                        replayed: true,
                    },
                ),
            )
        })
    }
}
#[test]
fn response_matches_shared_fixture_and_rejects_invalid_lineage() {
    let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent/conversation-fork.json"
    )))
    .unwrap();
    assert_eq!(
        serde_json::to_value(ForkResponse::try_from(saved()).unwrap()).unwrap(),
        expected
    );
    for end in [0, 9, u64::MAX] {
        let mut value = saved();
        value.parent_end_seq = end;
        assert!(matches!(
            ForkResponse::try_from(value),
            Err(CatalogCommandError::Corrupt)
        ));
    }
    let mut value = saved();
    value.parent_id = value.conversation.id;
    assert!(ForkResponse::try_from(value).is_err());
}
#[tokio::test]
async fn command_validates_scope_before_mutation_and_checks_returned_identity() {
    let store = Store {
        calls: Mutex::new(vec![]),
        result: saved(),
    };
    execute(&store, serde_json::from_value(request()).unwrap())
        .await
        .unwrap();
    assert_eq!(store.calls.lock().unwrap()[0].binding.resource_id, "note");
    for field in ["childId", "parentId", "throughTurnId"] {
        let mut value = request();
        value[field] = serde_json::json!(Uuid::nil());
        assert!(matches!(
            execute(&store, serde_json::from_value(value).unwrap()).await,
            Err(ForkCommandError::Domain(CatalogCommandError::InvalidInput))
        ));
    }
    let mut value = request();
    value["binding"]["resourceId"] = serde_json::json!(" ");
    assert!(
        execute(&store, serde_json::from_value(value).unwrap())
            .await
            .is_err()
    );
    assert_eq!(store.calls.lock().unwrap().len(), 1);
    let mut bad = saved();
    bad.through_turn_id = Uuid::from_u128(99);
    let wrong = Store {
        calls: Mutex::new(vec![]),
        result: bad,
    };
    assert!(matches!(
        execute(&wrong, serde_json::from_value(request()).unwrap()).await,
        Err(ForkCommandError::Domain(CatalogCommandError::Corrupt))
    ));
}

#[tokio::test]
async fn expected_epoch_never_falls_back_to_an_unfenced_store() {
    let store = Store {
        calls: Mutex::new(vec![]),
        result: saved(),
    };
    let mut fenced = request();
    fenced["expectedEpoch"] = serde_json::json!(Uuid::from_u128(99));
    assert!(
        execute(&store, serde_json::from_value(fenced.clone()).unwrap())
            .await
            .is_err()
    );
    assert!(store.calls.lock().unwrap().is_empty());
    fenced["expectedEpoch"] = serde_json::json!(Uuid::nil());
    assert!(matches!(
        execute(&store, serde_json::from_value(fenced).unwrap()).await,
        Err(ForkCommandError::Domain(CatalogCommandError::InvalidInput))
    ));
    assert!(store.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn command_dispatches_expected_epoch_and_the_complete_original_request_together() {
    struct FencedStore(Mutex<Vec<(ConversationForkRequest, Uuid)>>);
    impl ConversationForkStore for FencedStore {
        fn fork(&self, _: ConversationForkRequest) -> CatalogMutationFuture<'_, ConversationFork> {
            panic!("epoch fence must not be dropped")
        }
        fn fork_in_epoch(
            &self,
            request: ConversationForkRequest,
            epoch: Uuid,
        ) -> CatalogMutationFuture<'_, ConversationFork> {
            self.0.lock().unwrap().push((request, epoch));
            Box::pin(async { Ok(saved()) })
        }
    }
    let store = FencedStore(Mutex::new(vec![]));
    let epoch = Uuid::from_u128(99);
    let mut wire = request();
    wire["expectedEpoch"] = serde_json::json!(epoch);
    execute(&store, serde_json::from_value(wire).unwrap())
        .await
        .unwrap();
    let calls = store.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, epoch);
    assert_eq!(calls[0].0.parent_id, saved().parent_id);
    assert_eq!(calls[0].0.binding.resource_id, "note");
}
#[test]
fn request_rejects_extra_execution_options_and_unknown_hosts() {
    let mut extra = request();
    extra["startModel"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ForkRequest>(extra).is_err());
    let mut extra = request();
    extra["binding"]["permission"] = serde_json::json!("write");
    assert!(serde_json::from_value::<ForkRequest>(extra).is_err());
    let mut extra = request();
    extra["binding"]["kind"] = serde_json::json!("shell");
    assert!(serde_json::from_value::<ForkRequest>(extra).is_err());
}

#[tokio::test]
async fn status_uses_original_child_identity_and_never_invokes_fork() {
    let store = Store {
        calls: Mutex::new(vec![]),
        result: saved(),
    };
    let response = execute_status(&store, serde_json::from_value(request()).unwrap())
        .await
        .unwrap();
    let wire = serde_json::to_value(response).unwrap();
    assert_eq!(wire["operationId"], serde_json::json!(Uuid::from_u128(2)));
    assert_eq!(wire["conversationId"], wire["operationId"]);
    assert_eq!(wire["status"]["type"], "committed");
    assert_eq!(wire["status"]["proof"]["family"], "agent.conversation.fork");
    assert!(store.calls.lock().unwrap().is_empty());
}
