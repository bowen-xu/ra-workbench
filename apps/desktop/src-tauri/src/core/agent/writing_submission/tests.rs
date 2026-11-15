use super::*;
use crate::{
    agent::{
        WritingContextFile,
        repository::{SqliteSubmissionStore, create_conversation},
    },
    ai::JsonCredentialStore,
};
use research_agent_core::{conversation::ConversationId, submission::SubmissionStore};
use sha2::{Digest, Sha256};
#[cfg(unix)]
#[path = "tests/compile_synctex.rs"]
mod compile_synctex;
#[path = "tests/managed_fixture.rs"]
mod managed_fixture;
pub(super) use managed_fixture::{managed_fixture, managed_fixture_at};
#[path = "tests/queue_preview_request.rs"]
mod queue_preview_request;
pub(super) use queue_preview_request::queue_preview_intent;
#[cfg(unix)]
#[path = "tests/mcp_namespace.rs"]
mod mcp_namespace;
#[path = "tests/prepare_contract.rs"]
mod prepare_contract;
#[path = "tests/prepare_model_retention.rs"]
mod prepare_model_retention;
#[path = "tests/prepare_owned.rs"]
mod prepare_owned;
#[cfg(unix)]
#[path = "tests/runtime_context.rs"]
mod runtime_context;

#[tokio::test]
async fn configured_project_filters_primary_files_and_freezes_exact_write_paths() {
    use crate::agent::context_session::{
        permissions,
        sources::{Access, Part, Resource},
    };
    let f = fixture(true);
    let mut input = request(&f, Uuid::new_v4());
    let path = input.context.files[0].relative_path.clone();
    input.resources = vec![Resource::Manuscript {
        new_member_access: None,
        group_access: None,
        resource_id: f.manuscript.to_string(),
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: path.clone(),
            },
            label: path.clone(),
            selected: true,
            access: Access::ReadOnly,
        }],
    }];
    let service = WritingSubmissionService::from_app_store(&f.store);
    let read = service.prepare(&f.models, input).await.unwrap();
    assert_eq!(
        permissions::writing_paths(&read.record),
        Some(std::collections::HashSet::new())
    );
    let mut mixed = read.record.clone();
    let mut supplementary = mixed.input.context[0].clone();
    let mut anchor: serde_json::Value =
        serde_json::from_str(supplementary.anchor.as_deref().unwrap()).unwrap();
    anchor["contextOwner"]["resourceId"] = serde_json::json!(Uuid::new_v4());
    anchor["contextAccess"] = serde_json::json!("readWrite");
    anchor["supplementalContext"] = serde_json::json!(true);
    supplementary.anchor = Some(anchor.to_string());
    mixed.input.context.push(supplementary);
    assert_eq!(
        permissions::writing_paths(&mixed),
        Some(std::collections::HashSet::new())
    );
    let mut write = request(&f, Uuid::new_v4());
    write.resources = vec![Resource::Manuscript {
        new_member_access: None,
        group_access: None,
        resource_id: f.manuscript.to_string(),
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: path.clone(),
            },
            label: path.clone(),
            selected: true,
            access: Access::ReadWrite,
        }],
    }];
    let writable = service.prepare(&f.models, write).await.unwrap();
    assert_eq!(
        permissions::writing_paths(&writable.record),
        Some(std::collections::HashSet::from([path]))
    );
    assert_eq!(
        permissions::writing_paths(&read.record),
        Some(std::collections::HashSet::new())
    );
}

#[tokio::test]
async fn start_command_rejects_stale_disclosure_before_any_turn_is_created() {
    use crate::agent::{
        repository::SqliteConversationStore,
        service::AgentSessionService,
        writing_start::{self, StartError, StartRequest, WritingExecutionMode},
    };
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(AgentModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let sessions = std::sync::Arc::new(AgentSessionService::new(std::sync::Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let result = writing_start::start(
        &service,
        &f.models,
        sessions,
        StartRequest {
            conversation_id: f.conversation,
            request_id: prepared.record.request_id,
            manuscript_id: f.manuscript,
            context_snapshot_id: Uuid::new_v4(),
            model: prepared.record.input.model,
            mode: WritingExecutionMode::DeepSeekNonThinking,
        },
    )
    .await;
    assert!(matches!(result, Err(StartError::ConfirmationMismatch)));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        serde_json::from_value::<StartRequest>(serde_json::json!({
            "conversationId": f.conversation, "requestId": Uuid::new_v4(),
            "manuscriptId": f.manuscript, "contextSnapshotId": Uuid::new_v4(),
            "model": {"providerId":"deepseek","modelId":"deepseek-v4-pro","revision":null},
            "mode":"deepSeekNonThinking", "maxOutputTokens":1000000
        }))
        .is_err()
    );
}

#[tokio::test]
async fn command_preview_serializes_only_frozen_input_and_recovers_original_receipt() {
    use crate::agent::writing_commands::{self, PrepareRequest, RestoreRequest};
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let input = request(&f, Uuid::new_v4());
    let request_id = input.request_id;
    let preview = writing_commands::prepare(
        &service,
        &f.models,
        PrepareRequest {
            resources: vec![],
            user_input: vec![],
            conversation_id: input.conversation_id,
            request_id,
            text: input.text,
            context: input.context,
            retrieval: None,
            model: None,
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_value(&preview).unwrap();
    assert_eq!(json["input"]["context"][0]["text"], "Frozen manuscript");
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json.as_object().unwrap().len(), 7);
    assert!(!json.to_string().contains("synthetic-fixture"));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let restored = writing_commands::restore(
        &service,
        &f.models,
        RestoreRequest {
            conversation_id: f.conversation,
            request_id,
            manuscript_id: f.manuscript,
        },
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), json);
    assert!(
        serde_json::from_value::<RestoreRequest>(serde_json::json!({
            "conversationId": f.conversation, "requestId": request_id,
            "manuscriptId": f.manuscript, "capabilities": {"tools": true}
        }))
        .is_err()
    );
}
pub(crate) struct Fixture {
    pub(super) root: tempfile::TempDir,
    pub(crate) store: AppStore,
    pub(crate) models: AgentModelResolver,
    pub(super) conversation: Uuid,
    pub(crate) manuscript: Uuid,
    pub(super) hash: String,
}
pub(crate) fn fixture(bound: bool) -> Fixture {
    managed_fixture(bound)
}
pub(super) fn legacy_fixture(bound: bool) -> Fixture {
    fixture_at(bound, tempfile::tempdir().unwrap())
}
#[tokio::test]
async fn supplementary_note_mentions_record_read_only_resource_grants() {
    let f = fixture(true);
    let note = Uuid::new_v4();
    f.store.connection().unwrap().execute(
        "insert into note_pages(id,title,created_at,updated_at) values (?1,'Saved supplementary note','now','now')",
        [note.to_string()],
    ).unwrap();
    let mut input = request(&f, Uuid::new_v4());
    input.resources =
        vec![crate::agent::context_session::sources::Resource::Note { resource_id: note }];
    input.user_input = serde_json::from_value(serde_json::json!([{"type":"text", "text":input.text, "textElements":[]}, {
        "type": "mention", "name": "Saved supplementary note", "target": {
            "kind": "file", "sourceId": format!("context:{}", serde_json::json!({"kind":"note","resourceId":note})),
            "resourceId": note.to_string()
        }
    }])).unwrap();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = input.request_id;
    let prepared = service.prepare(&f.models, input).await.unwrap();
    assert_eq!(prepared.record.input.context.len(), 2);
    let extra = &prepared.record.input.context[1];
    assert!(extra.text.contains("Saved supplementary note"));
    assert!(crate::agent::context_session::resource_grants::resource(extra).is_some());
    assert!(
        extra
            .anchor
            .as_ref()
            .unwrap()
            .contains("supplementalContext")
    );
    let mention = serde_json::to_value(&prepared.record.input.user_input[1]).unwrap();
    assert_eq!(mention["target"]["sourceId"], extra.source_id);
    assert_eq!(mention["target"]["resourceId"], note.to_string());
    assert!(mention["target"].get("revision").is_none());
    assert!(extra.revision.starts_with("grant:"));
    let anchor: serde_json::Value = serde_json::from_str(extra.anchor.as_deref().unwrap()).unwrap();
    assert_eq!(anchor["contextAccess"], "readOnly");
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='Changed later',revision=revision+1 where id=?1",
            [note.to_string()],
        )
        .unwrap();
    let restored = service
        .restore(
            &f.models,
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                manuscript_id: f.manuscript,
                request_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(prepared.record.input, restored.record.input);
}
pub(super) fn fixture_at(bound: bool, root: tempfile::TempDir) -> Fixture {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 123);
    let conversation = Uuid::new_v4();
    let manuscript = Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(conversation), "Writing").unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values ('project','fixture','','now')", []).unwrap();
    db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','fixture','main.tex','now','now')", [manuscript.to_string()]).unwrap();
    if bound {
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)", params![conversation.to_string(), manuscript.to_string()]).unwrap();
    }
    db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('deepseek',1,'',60000,'now') on conflict(provider_id) do update set enabled=1,base_url=''", []).unwrap();
    db.execute("insert into llm_model_routes(route_kind,route_id,model_kind,provider_id,model_id,updated_at) values ('default','text','text','deepseek','fixture-model','now') on conflict(route_kind,route_id) do update set provider_id='deepseek',model_id='fixture-model'", []).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
    db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,'main.tex','tex',17,?2,'now')", params![manuscript.to_string(), hash]).unwrap();
    let store = AppStore::for_test(db, root.path().join("data/objects"));
    let source = store
        .manuscript_directory()
        .join(manuscript.to_string())
        .join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
    let credentials = JsonCredentialStore::from_path(root.path().join("credentials.json"));
    credentials.set("deepseek", "synthetic-fixture").unwrap();
    let models = AgentModelResolver::from_app_store(&store, credentials);
    Fixture {
        root,
        store,
        models,
        conversation,
        manuscript,
        hash,
    }
}
pub(crate) fn request(f: &Fixture, id: Uuid) -> WritingSubmissionRequest {
    WritingSubmissionRequest {
        resources: vec![],
        user_input: vec![],
        conversation_id: f.conversation,
        request_id: id,
        text: "Explain".into(),
        model: None,
        session_config: None,
        turn_config: None,
        context: WritingContextRequest {
            manuscript_id: f.manuscript,
            files: vec![WritingContextFile {
                relative_path: "main.tex".into(),
                expected_disk_sha256: f.hash.clone(),
                buffer: None,
                selection: None,
            }],
        },
        retrieval: None,
    }
}
fn count(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_submissions", [], |r| r.get(0))
        .unwrap()
}
#[tokio::test]
async fn receipt_restore_uses_original_source_and_model_after_files_and_defaults_change() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = Uuid::new_v4();
    let prepared = service
        .prepare(&f.models, request(&f, request_id))
        .await
        .unwrap();
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("update llm_model_routes set model_id='changed-default'", [])
        .unwrap();
    let restored = service
        .restore(
            &f.models,
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id,
                manuscript_id: f.manuscript,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.record, prepared.record);
    assert_eq!(restored.model.identity().model_id, "fixture-model");
    assert_eq!(count(&f), 1);
}
#[tokio::test]
async fn restore_rechecks_current_binding_and_original_snapshot_scope() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = Uuid::new_v4();
    service
        .prepare(&f.models, request(&f, request_id))
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id,
        manuscript_id: f.manuscript,
    };
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_submissions set input_json=json_set(input_json,'$.context[0].anchor',?1)",
            [serde_json::json!({"manuscriptId":Uuid::new_v4()}).to_string()],
        )
        .unwrap();
    assert!(matches!(
        service.restore(&f.models, receipt).await,
        Err(WritingSubmissionError::Unbound)
    ));
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(matches!(
        service.restore(&f.models, receipt).await,
        Err(WritingSubmissionError::Unbound)
    ));
    assert_eq!(count(&f), 1);
}
#[tokio::test]
async fn missing_credentials_do_not_rewrite_or_replace_original_receipt() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = Uuid::new_v4();
    let original = service
        .prepare(&f.models, request(&f, request_id))
        .await
        .unwrap()
        .record;
    JsonCredentialStore::from_path(f.root.path().join("credentials.json"))
        .delete("deepseek")
        .unwrap();
    assert!(matches!(
        service
            .restore(
                &f.models,
                WritingSubmissionReceipt {
                    conversation_id: f.conversation,
                    request_id,
                    manuscript_id: f.manuscript
                }
            )
            .await,
        Err(WritingSubmissionError::Model(
            AgentModelPreparationError::Credentials
        ))
    ));
    assert_eq!(
        SqliteSubmissionStore::new(f.store.shared_agent_connection())
            .get(f.conversation, request_id)
            .await
            .unwrap(),
        original
    );
}

#[cfg(unix)]
#[path = "tests/instructions.rs"]
mod instructions;

#[path = "tests/dynamic_scope.rs"]
mod dynamic_scope;
#[path = "tests/text_patch.rs"]
mod text_patch;
