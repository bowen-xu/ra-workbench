use super::*;
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent/conversation-deletion.json"
    ))
    .unwrap()
}
#[test]
fn deletion_wire_matches_shared_fixture_and_error_vocabulary() {
    let fixture = fixture();
    let request = serde_json::from_value::<DeleteRequest>(fixture["request"].clone())
        .unwrap()
        .decode()
        .unwrap();
    let response = DeleteResponse::from(ConversationDeletionReceipt {
        request_id: request.request_id,
        conversation_id: request.conversation_id,
    });
    assert_eq!(serde_json::to_value(response).unwrap(), fixture["response"]);
    let errors = [
        DeleteError::InvalidInput,
        DeleteError::NotFound,
        DeleteError::Conflict,
        DeleteError::Busy,
        DeleteError::TooLarge,
        DeleteError::Corrupt,
        DeleteError::Unavailable,
        DeleteError::NotArchived,
        DeleteError::BranchHistory,
        DeleteError::ProposalApplications,
        DeleteError::ExecutionWorkspace,
    ];
    assert_eq!(serde_json::to_value(errors).unwrap(), fixture["errors"]);
}
#[test]
fn deletion_rejects_unknown_fields_invalid_scope_and_confirmation_before_admission() {
    let valid = fixture()["request"].clone();
    for (field, value) in [
        ("schemaVersion", serde_json::json!(2)),
        ("requestId", serde_json::json!(Uuid::nil())),
        ("conversationId", serde_json::json!(Uuid::nil())),
        ("expectedTitle", serde_json::json!(" ")),
        ("expectedTitle", serde_json::json!("a".repeat(181))),
        ("expectedTitle", serde_json::json!("a\0")),
        (
            "expectedDurableCount",
            serde_json::json!(9_007_199_254_740_992_u64),
        ),
    ] {
        let mut value_request = valid.clone();
        value_request[field] = value;
        assert_eq!(
            serde_json::from_value::<DeleteRequest>(value_request)
                .unwrap()
                .decode()
                .unwrap_err(),
            DeleteError::InvalidInput
        );
    }
    assert!(
        !serde_json::from_value::<DeleteRequest>(valid.clone())
            .unwrap()
            .decode()
            .unwrap()
            .force
    );
    let mut forced = valid.clone();
    forced["force"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<DeleteRequest>(forced)
            .unwrap()
            .decode()
            .unwrap()
            .force
    );
    for invalid in [
        serde_json::json!("true"),
        serde_json::json!(1),
        serde_json::Value::Null,
    ] {
        let mut value = valid.clone();
        value["force"] = invalid;
        assert!(serde_json::from_value::<DeleteRequest>(value).is_err());
    }
    for field in ["deleteFiles", "startModel"] {
        let mut value = valid.clone();
        value[field] = serde_json::json!(true);
        assert!(serde_json::from_value::<DeleteRequest>(value).is_err());
    }
    for resource in ["".to_owned(), "x".repeat(257), "x\0".to_owned()] {
        let mut value = valid.clone();
        value["binding"]["resourceId"] = serde_json::json!(resource);
        assert!(
            serde_json::from_value::<DeleteRequest>(value)
                .unwrap()
                .decode()
                .is_err()
        );
    }
}

#[test]
fn deletion_storage_errors_keep_the_original_operation_and_safe_codes() {
    use research_agent_core::conversation::{
        CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic,
    };
    let operation = Uuid::new_v4();
    let error = DeleteError::from(ConversationDeletionError::Data(CatalogDataFailure {
        kind: CatalogDataFailureKind::Unknown,
        diagnostic: Box::new(CatalogMutationDiagnostic {
            operation_id: operation,
            epoch: Some(Uuid::new_v4()),
            sql_fence: Some("18446744073709551615".into()),
            phase: "commit",
            sqlite_code: Some("ConstraintViolation".into()),
            sqlite_extended_code: Some(787),
            safe_cause: "sql_commit_outcome_unknown",
        }),
    }));
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["code"], "storage");
    assert_eq!(wire["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        wire["reason"]["reason"]["operationId"],
        operation.to_string()
    );
    assert_eq!(wire["reason"]["reason"]["sqliteExtendedCode"], 787);
}
#[tokio::test]
async fn deletion_status_uses_the_original_request_without_dispatching_delete() {
    use research_agent_core::conversation::{
        CatalogMutationFuture, CatalogMutationStatus, ConversationDeletionFuture,
    };
    struct ReadOnly;
    impl ConversationDeletionStore for ReadOnly {
        fn delete(&self, _: ConversationDeletionRequest) -> ConversationDeletionFuture<'_> {
            panic!("status must never delete")
        }
        fn deletion_status(
            &self,
            request: ConversationDeletionRequest,
        ) -> CatalogMutationFuture<'_, CatalogMutationStatus> {
            request.validate().unwrap();
            Box::pin(async {
                Ok(CatalogMutationStatus::NoSqlReceipt {
                    epoch: Uuid::from_u128(9),
                    sql_fence: 7,
                    closed: false,
                })
            })
        }
    }
    let expected = fixture()["request"].clone();
    let request = serde_json::from_value::<DeleteRequest>(expected.clone()).unwrap();
    let wire = serde_json::to_value(execute_status(&ReadOnly, request).await.unwrap()).unwrap();
    assert_eq!(wire["operationId"], expected["requestId"]);
    assert_eq!(wire["conversationId"], expected["conversationId"]);
    assert_eq!(wire["status"]["type"], "noSqlReceipt");
    assert_eq!(wire["status"]["sqlFence"], "7");
}

#[tokio::test]
async fn deletion_epoch_rejects_before_runtime_admission_and_preserves_the_old_request_shape() {
    use crate::agent::repository::SqliteConversationStore;
    use crate::data_interface::Database;
    use rusqlite::Connection;
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = Database::open(sql, &root.path().join("log")).unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "Synthetic".into()).unwrap(),
    )));
    let mut raw = fixture()["request"].clone();
    let id = ConversationId::from_uuid(
        Uuid::parse_str(raw["conversationId"].as_str().unwrap()).unwrap(),
    );
    // Reaching runtime admission would report Busy; the epoch must win first.
    let _reserved = service.tasks().reserve_idle(id).unwrap();
    let before = db.benchmark_snapshot().unwrap();
    let changes = db.lock().unwrap().total_changes();
    raw["expectedEpoch"] = serde_json::json!(Uuid::new_v4());
    let error = execute(
        &service,
        &catalog,
        serde_json::from_value(raw.clone()).unwrap(),
    )
    .await
    .unwrap_err();
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert_eq!(wire["reason"]["reason"]["phase"], "delete-epoch");
    assert_eq!(db.benchmark_snapshot().unwrap(), before);
    assert_eq!(db.lock().unwrap().total_changes(), changes);
    raw["expectedEpoch"] = serde_json::json!(Uuid::nil());
    assert_eq!(
        serde_json::from_value::<DeleteRequest>(raw)
            .unwrap()
            .decode()
            .unwrap_err(),
        DeleteError::InvalidInput
    );
}
