use super::*;
use crate::agent::repository::submissions::insert_in;
use research_agent_core::protocol::ModelIdentity;

fn database() -> Connection {
    populate_database(Connection::open_in_memory().unwrap())
}

#[test]
fn managed_storage_failure_uses_shared_safe_diagnostic_envelope() {
    let operation = Uuid::new_v4();
    let epoch = Uuid::new_v4();
    let failure = Error::Storage(Box::new(StorageFailure {
        code: "DATA_SQL_COMMIT_UNKNOWN",
        operation_id: operation,
        epoch: Some(epoch),
        sql_fence: Some("9007199254740993".into()),
        phase: "commit",
        safe_cause: "sql_commit_outcome_unknown",
        sqlite_code: Some("ConstraintViolation".into()),
        sqlite_extended_code: Some(787),
    }));
    let value = serde_json::to_value(failure).unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["reason"]["operationId"],
        operation.to_string()
    );
    assert_eq!(value["reason"]["reason"]["epoch"], epoch.to_string());
    assert_eq!(value["reason"]["reason"]["sqlFence"], "9007199254740993");
    assert_eq!(value["reason"]["reason"]["sqliteExtendedCode"], 787);
    assert_eq!(
        serde_json::to_value(Error::InvalidInput).unwrap(),
        "invalidInput"
    );
}

fn populate_database(mut db: Connection) -> Connection {
    crate::storage::migrate(&mut db).unwrap();
    db.execute(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
         values('paper','journalArticle','Fixture','now','now')",
        [],
    )
    .unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Reading')",
        [Uuid::from_u128(1).to_string()],
    )
    .unwrap();
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id)
         values(?1,'paper','paper')",
        [Uuid::from_u128(1).to_string()],
    )
    .unwrap();
    db
}

#[tokio::test]
async fn disk_reopen_restores_frozen_profile_without_reading_changed_configuration() {
    use crate::agent::config::{profile_snapshot, profiles::ProfileKind};
    use std::fs;
    let root = tempfile::tempdir().unwrap();
    let database_path = root.path().join("fixture.sqlite");
    let objects = root.path().join("data/objects");
    let store = AppStore::for_test(
        populate_database(Connection::open(&database_path).unwrap()),
        objects.clone(),
    );
    let config_path = store
        .agent_configuration_directory()
        .join("agent/config.toml");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        "schema_version=1\n[profiles.reading]\ntext='Original reading instructions'\n",
    )
    .unwrap();
    let mut input = record().input;
    (input.instructions, input.instruction_configs) = profile_snapshot::freeze(
        store.agent_configuration_directory(),
        ProfileKind::Reading,
        "paper:paper".into(),
        [
            Some(research_agent_core::config::InstructionConfigPatch {
                max_bytes: Some(16),
                fallback_filenames: None,
            }),
            Some(research_agent_core::config::InstructionConfigPatch {
                max_bytes: Some(8),
                fallback_filenames: None,
            }),
        ],
    )
    .unwrap();
    let frozen = {
        let mut db = store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let frozen = insert_in(&tx, Uuid::from_u128(1), Uuid::from_u128(2), input).unwrap();
        tx.commit().unwrap();
        frozen
    };
    fs::write(&config_path, "invalid configuration").unwrap();
    assert!(
        profile_snapshot::freeze(
            store.agent_configuration_directory(),
            ProfileKind::Reading,
            "paper:paper".into(),
            [None, None]
        )
        .is_err()
    );
    drop(store);
    let reopened = AppStore::for_test(Connection::open(database_path).unwrap(), objects);
    let receipt = Receipt {
        conversation_id: frozen.conversation_id,
        request_id: frozen.request_id,
        paper_id: "paper".into(),
        paper_context_snapshot_id: "paper-context-snapshot".into(),
    };
    assert_eq!(restore(&reopened, &receipt).await.unwrap(), frozen);
    let mut foreign = frozen;
    foreign.input.instructions[0].scope_id = "paper:another".into();
    assert_eq!(
        validate_record(&foreign, "paper", "paper-context-snapshot"),
        Err(Error::Unavailable)
    );
}

fn record() -> SubmissionRecord {
    SubmissionRecord {
        conversation_id: Uuid::from_u128(1),
        request_id: Uuid::from_u128(2),
        turn_id: Uuid::from_u128(3),
        context_snapshot_id: Uuid::from_u128(4),
        input: SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            text: "Explain the result".into(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "vision".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![research_agent_core::submission::ContextText {
                source_id: "paper-source:one".into(),
                kind: ContextSourceKind::Paper,
                resource_id: "paper".into(),
                revision: "a".repeat(64),
                anchor: Some(
                    serde_json::json!({
                        "paperId": "paper",
                        "snapshotId": "paper-context-snapshot",
                        "attachmentId": "attachment",
                        "parseRunId": "parse",
                        "kind": "text",
                        "rawRef": "#/texts/0",
                        "pageIndex": 0
                    })
                    .to_string(),
                ),
                text: "Evidence".into(),
            }],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        },
    }
}

#[test]
fn binding_requires_active_conversation_and_untrashed_paper() {
    let db = database();
    let conversation = Uuid::from_u128(1);
    assert_eq!(check_binding(&db, conversation, "paper"), Ok(()));
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert_eq!(
        check_binding(&db, conversation, "paper"),
        Err(Error::Unbound)
    );
    db.execute("update agent_conversations set archived=0", [])
        .unwrap();
    db.execute(
        "update bibliographic_items set trashed_at='now' where id='paper'",
        [],
    )
    .unwrap();
    assert_eq!(
        check_binding(&db, conversation, "paper"),
        Err(Error::Unbound)
    );
}

#[test]
fn restored_input_must_retain_paper_and_snapshot_identity() {
    let valid = record();
    assert_eq!(
        validate_record(&valid, "paper", "paper-context-snapshot"),
        Ok(())
    );
    assert_eq!(
        validate_record(&valid, "another-paper", "paper-context-snapshot"),
        Err(Error::Unbound)
    );
    assert_eq!(
        validate_record(&valid, "paper", "another-snapshot"),
        Err(Error::Unbound)
    );
    let mut wrong_kind = valid;
    wrong_kind.input.context[0].kind = ContextSourceKind::Note;
    assert_eq!(
        validate_record(&wrong_kind, "paper", "paper-context-snapshot"),
        Err(Error::Unbound)
    );
}

#[tokio::test]
async fn image_only_paper_restore_checks_the_original_scope() {
    use sha2::{Digest, Sha256};
    let mut db = database();
    db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('paper-context-snapshot','paper',?1,1,'now')", ["a".repeat(64)]).unwrap();
    let mut input = record().input;
    input.context.clear();
    let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
    let image = research_agent_core::submission::SubmissionImage {
        source_id: "paper-source:fixture".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes,
    };
    let tx = db.transaction().unwrap();
    let saved = crate::agent::repository::submissions::insert_in_with_images(
        &tx,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        input,
        &[image],
    )
    .unwrap();
    tx.execute("insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,'paper','paper-context-snapshot',?4)", params![saved.conversation_id.to_string(), saved.request_id.to_string(), saved.turn_id.to_string(), "a".repeat(64)]).unwrap();
    tx.commit().unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let mut receipt = Receipt {
        conversation_id: saved.conversation_id,
        request_id: saved.request_id,
        paper_id: "paper".into(),
        paper_context_snapshot_id: "paper-context-snapshot".into(),
    };
    assert_eq!(restore(&store, &receipt).await.unwrap(), saved);
    receipt.paper_context_snapshot_id = "other".into();
    assert_eq!(restore(&store, &receipt).await.unwrap_err(), Error::Unbound);
}

#[test]
fn native_source_failure_preserves_original_sql_payload_and_distinct_request_identity() {
    use crate::{
        hbs_storage::boundary::HbsOutcome,
        runtime_plugins::{
            literature_hbs::HbsOperationError,
            native::{NativeHostError, NativeProfile},
        },
    };
    let conversation = Uuid::new_v4();
    let request = Uuid::new_v4();
    let native_request = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let input: PrepareRequest = serde_json::from_value(serde_json::json!({
        "conversationId":conversation,"requestId":request,"paperId":"paper",
        "paperContextSnapshotId":"snapshot","expectedContextHash":"a".repeat(64),"text":"read","model":null
    })).unwrap();
    let mut native = NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_exited",
        "文献宿主已退出。",
    );
    native.package_key = Some("a".repeat(64));
    native.instance_id = Some(instance.to_string());
    native.request_id = Some(native_request.to_string());
    let (message, diagnostic) =
        HbsOperationError::native(native, "source-read", HbsOutcome::Unknown).into_wire();
    let source = SourceFailure::attach(
        Error::Storage(Box::new(StorageFailure {
            code: "DATA_SQL_COMMIT_UNKNOWN",
            operation_id: request,
            epoch: None,
            sql_fence: None,
            phase: "rollback",
            safe_cause: "sql_rollback_failed",
            sqlite_code: Some("ConstraintViolation".into()),
            sqlite_extended_code: Some(787),
        })),
        message,
        diagnostic,
        None,
        &input,
    );
    let value = serde_json::to_value(source).unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["reason"]["operationId"],
        request.to_string()
    );
    assert_eq!(
        value["reason"]["reason"]["safeCause"],
        "sql_rollback_failed"
    );
    assert_eq!(value["reason"]["reason"]["sqliteExtendedCode"], 787);
    assert_eq!(value["paperContextOperation"], "submission");
    assert_eq!(value["conversationId"], conversation.to_string());
    assert_eq!(value["requestId"], request.to_string());
    assert_eq!(value["paperId"], "paper");
    assert_eq!(value["paperContextSnapshotId"], "snapshot");
    assert_eq!(value["nativeFailure"]["packageKey"], "a".repeat(64));
    assert_eq!(value["nativeFailure"]["instanceId"], instance.to_string());
    assert_eq!(
        value["nativeFailure"]["nativeRequestId"],
        native_request.to_string()
    );
}
