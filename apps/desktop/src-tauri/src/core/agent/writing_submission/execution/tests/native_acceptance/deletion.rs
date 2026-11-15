//! Delete only a retained synthetic fixture after its native process has exited.
use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationCatalog, ConversationDeletionRequest,
};
use std::path::{Path, PathBuf};

fn footprint(db: &Connection) -> Vec<String> {
    [
        "select json_group_array(json_object('id',id,'change',change_set_id,'direction',direction,'status',status)) from (select * from agent_proposal_applications order by id)",
        "select json_group_array(json_object('application',application_id,'path',relative_path,'phase',phase,'session',expected_editor_session,'revision',expected_editor_revision,'result',result_editor_revision)) from (select * from agent_proposal_application_files order by application_id,relative_path)",
        "select json_group_array(json_object('change',change_set_id,'path',relative_path,'before',before_sha256,'after',after_sha256,'source',source_json)) from (select * from agent_change_set_files order by change_set_id,relative_path)",
        "select json_group_array(json_object('path',relative_path,'revision',revision,'checkpoint',checkpoint_json)) from (select * from writing_document_checkpoints order by relative_path)",
    ]
    .map(|sql| db.query_row(sql, [], |row| row.get::<_, String>(0)).unwrap())
    .into()
}

#[tokio::test]
#[ignore = "Deletes source chat only in an explicitly supplied, stopped synthetic native fixture; run with --exact --ignored --nocapture"]
async fn delete_source_of_pending_native_application() {
    delete_pending_source(PendingApplicationState::AllPending).await;
}

#[tokio::test]
#[ignore = "Deletes source chat only in an explicitly supplied, stopped synthetic partially applied native fixture; run with --exact --ignored --nocapture"]
async fn delete_source_of_partially_applied_native_application() {
    delete_pending_source(PendingApplicationState::PartiallyApplied).await;
}

enum PendingApplicationState {
    AllPending,
    PartiallyApplied,
}

async fn delete_pending_source(state: PendingApplicationState) {
    let partial = matches!(state, PendingApplicationState::PartiallyApplied);
    let root = PathBuf::from(
        std::env::var("RESEARCH_ASSISTANT_NATIVE_PENDING_SOURCE_DIRECTORY")
            .expect("explicit synthetic fixture directory is required"),
    );
    assert_eq!(root.canonicalize().unwrap(), root);
    assert_eq!(root.parent(), Some(Path::new("/private/tmp")));
    assert!(
        root.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("research-assistant-writing-diff-agent-")
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("native-acceptance.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["dataDirectory"].as_str(), root.to_str());
    let conversation = Uuid::parse_str(manifest["conversationId"].as_str().unwrap()).unwrap();
    let manuscript = Uuid::parse_str(manifest["manuscriptId"].as_str().unwrap()).unwrap();
    let db = Connection::open(root.join("research-assistant.sqlite3")).unwrap();
    let store = AppStore::for_test(db, root.join("objects"));
    let before = {
        let db = store.connection().unwrap();
        let state: (i64, i64, i64) = db.query_row(
            "select (select count(*) from agent_proposal_applications where direction='apply' and status='applying'),(select count(*) from agent_proposal_application_files where phase='pending' and result_editor_revision is null),(select count(*) from agent_proposal_application_files)",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(state, (1, if partial { 1 } else { 2 }, 2));
        if partial {
            let state: (String, String, String) = db.query_row(
                "select relative_path,phase,result_editor_revision from agent_proposal_application_files where phase='acknowledged'",
                [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).unwrap();
            assert_eq!(state.0, "main.tex");
            assert_eq!(state.1, "acknowledged");
            assert!(!Uuid::parse_str(&state.2).unwrap().is_nil());
        }
        footprint(&db)
    };
    let source = store
        .manuscript_directory()
        .join(manuscript.to_string())
        .join("source");
    for (path, key) in [
        ("main.tex", if partial { "mainAfter" } else { "mainBefore" }),
        ("sections/introduction.tex", "introductionBefore"),
    ] {
        assert_eq!(
            std::fs::read_to_string(source.join(path)).unwrap(),
            manifest[key].as_str().unwrap()
        );
    }
    let catalog = Arc::new(SqliteConversationCatalog::from_app_store(&store));
    let id = ConversationId::from_uuid(conversation);
    let summary = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(research_agent_core::conversation::ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(summary.durable_count),
        })
        .await
        .unwrap();
    let summary = catalog.read(id, None).await.unwrap();
    assert_eq!(summary.title, "Native Agent acceptance");
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "native-acceptance".into())
            .unwrap(),
    )));
    let receipt = sessions
        .delete_conversation(
            catalog.clone(),
            ConversationDeletionRequest {
                request_id: Uuid::new_v4(),
                conversation_id: id,
                binding: Some(ConversationBinding {
                    kind: BindingKind::Manuscript,
                    resource_id: manuscript.to_string(),
                }),
                expected_title: summary.title,
                expected_durable_count: summary.durable_count,
                force: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(receipt.conversation_id, id);
    assert!(catalog.read(id, None).await.is_err());
    assert_eq!(footprint(&store.connection().unwrap()), before);
    for (path, key) in [
        ("main.tex", if partial { "mainAfter" } else { "mainBefore" }),
        ("sections/introduction.tex", "introductionBefore"),
    ] {
        assert_eq!(
            std::fs::read_to_string(source.join(path)).unwrap(),
            manifest[key].as_str().unwrap()
        );
    }
    sessions.shutdown().await.unwrap();
    println!(
        "Deleted synthetic source; pending application, frozen files, checkpoints and formal text unchanged"
    );
}
