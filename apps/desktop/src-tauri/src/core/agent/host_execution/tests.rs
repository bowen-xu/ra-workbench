use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::ModelIdentity,
    submission::{ContextSourceKind, ContextText, SubmissionInput},
};
pub(crate) fn fixture(kind: &str) -> (tempfile::TempDir, AppStore, SubmissionRecord) {
    fixture_with_data_interface(kind, false)
}
#[cfg(target_os = "macos")]
pub(crate) fn managed_fixture(kind: &str) -> (tempfile::TempDir, AppStore, SubmissionRecord) {
    fixture_with_data_interface(kind, true)
}
fn fixture_with_data_interface(
    kind: &str,
    managed: bool,
) -> (tempfile::TempDir, AppStore, SubmissionRecord) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 143);
    let conversation = Uuid::new_v4();
    super::super::repository::create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation),
        "Scripts",
    )
    .unwrap();
    let id = Uuid::new_v4().to_string();
    match kind {
        "paper" => {
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle','Fixture','now','now')",[&id]).unwrap();
        }
        "note" => {
            db.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Fixture','now','now')",[&id]).unwrap();
        }
        "readingNote" => {
            crate::notes::insert_reading_fixture(&db, &id, "Script source");
        }
        _ => panic!("invalid fixture"),
    }
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,?2,?3)",params![conversation.to_string(),kind,id]).unwrap();
    let transaction = db.transaction().unwrap();
    let record = super::super::repository::submissions::insert_in(
        &transaction,
        conversation,
        Uuid::new_v4(),
        SubmissionInput {
            remote_compaction_v2: None,
            text: "Run skill script".into(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![ContextText {
                source_id: "source".into(),
                resource_id: id.clone(),
                kind: ContextSourceKind::Note,
                revision: "1".into(),
                anchor: None,
                text: "frozen evidence".into(),
            }],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            user_input: vec![],
            skill_instructions: vec![],
        },
    )
    .unwrap();
    transaction.commit().unwrap();
    db.execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values(?1,?2,?3,'r1')",params![conversation.to_string(),kind,id]).unwrap();
    let content = root.path().canonicalize().unwrap().join("data/objects");
    let store = if managed {
        crate::storage::migrate(&mut db).unwrap();
        let owner =
            crate::data_interface::Database::open(db, &root.path().join("agent-log")).unwrap();
        AppStore::for_test_database(owner, content)
    } else {
        AppStore::for_test(db, content)
    };
    std::fs::create_dir_all(store.agent_configuration_directory()).unwrap();
    (root, store, record)
}
#[test]
fn all_hosts_freeze_grants_and_cannot_gain_new_permissions_mid_turn() {
    for kind in ["paper", "note", "readingNote"] {
        let (_root, store, record) = fixture(kind);
        let db = store.connection().unwrap();
        crate::agent::skills::steering::freeze_grants(&db, &store, &record).unwrap();
        assert_eq!(grant(&db, &record).unwrap().as_deref(), Some("r1"));
        db.execute("update agent_host_shell_grants set revision='r2'", [])
            .unwrap();
        assert_eq!(grant(&db, &record).unwrap(), None);
        crate::agent::skills::steering::freeze_grants(&db, &store, &record).unwrap();
        assert_eq!(grant(&db, &record).unwrap(), None);
        db.execute("update agent_host_shell_grants set revision='r1'", [])
            .unwrap();
        db.execute("delete from agent_conversation_bindings", [])
            .unwrap();
        assert_eq!(grant(&db, &record).unwrap(), None);
    }
}
#[cfg(target_os = "macos")]
#[tokio::test]
async fn each_host_runs_in_a_private_copy_and_cancels_owned_processes() {
    use research_agent_core::{
        model::ToolCall,
        runner::{RunFuture, ToolExecutor, ToolOutput},
        tool_registry::OwnedToolRegistration,
    };
    struct Empty;
    impl ToolExecutor for Empty {
        fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
            Ok(())
        }
        fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
            Box::pin(async { Err(RunError::ToolUnavailable) })
        }
    }
    for kind in ["paper", "note", "readingNote"] {
        let (root, store, mut record) = fixture(kind);
        let skill = store
            .agent_configuration_directory()
            .join(".agents/skills/fixture");
        std::fs::create_dir_all(skill.join("scripts")).unwrap();
        std::fs::create_dir_all(skill.join("assets")).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\ndescription: Execute the fixture script\n---\nRun scripts/run.sh with the frozen context.").unwrap();
        std::fs::write(skill.join("scripts/run.sh"), "cat context/0.txt; cat \"$(dirname \"$0\")/../assets/evidence.txt\"; printf copy > output.txt").unwrap();
        std::fs::write(skill.join("assets/evidence.txt"), "skill asset").unwrap();
        let (skills, prompts) = crate::agent::skills::injection::freeze_application(
            store.agent_configuration_directory(),
            "$fixture",
            &[],
        )
        .unwrap();
        let package = crate::agent::skills::packages::relative(&skills[0].descriptor.identity);
        record.input.skills = skills;
        record.input.skill_instructions = prompts;
        {
            let db = store.connection().unwrap();
            crate::agent::skills::steering::freeze_grants(&db, &store, &record).unwrap();
        }
        let sessions = std::sync::Arc::new(
            crate::agent::service::AgentSessionService::from_app_store(&store).unwrap(),
        );
        sessions
            .processes()
            .begin_turn(record.conversation_id, record.turn_id)
            .unwrap();
        let tools = registration(
            store.clone(),
            record.clone(),
            sessions.clone(),
            OwnedToolRegistration::new(vec![], std::sync::Arc::new(Empty)),
        )
        .await
        .unwrap();
        let (_, executor) = tools.into_parts();
        let exec = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_exec_command".into(),
            arguments: serde_json::json!({"command":format!("sh {package}/scripts/run.sh"),"yield_ms":5000}),
        };
        let result = executor.authorize_and_execute(exec.clone()).await.unwrap();
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("frozen evidence"));
        assert!(result.content.contains("skill asset"));
        executor.result_committed(exec).await.unwrap();
        assert!(!root.path().join("output.txt").exists());
        let attempt = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_exec_command".into(),
            arguments: serde_json::json!({"command":format!("printf tamper > {package}/assets/evidence.txt"),"yield_ms":5000}),
        };
        let denied = executor
            .authorize_and_execute(attempt.clone())
            .await
            .unwrap();
        assert!(
            denied.is_error || denied.content.contains("Operation not permitted"),
            "{}",
            denied.content
        );
        executor.result_committed(attempt).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(skill.join("assets/evidence.txt")).unwrap(),
            "skill asset"
        );
        let long = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_exec_command".into(),
            arguments: serde_json::json!({"command":"sleep 60","yield_ms":0}),
        };
        let result = executor.authorize_and_execute(long.clone()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["state"], "running");
        executor.result_committed(long).await.unwrap();
        let cancel = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_process".into(),
            arguments: serde_json::json!({"action":"cancel","process_id":payload["processId"]}),
        };
        executor
            .authorize_and_execute(cancel.clone())
            .await
            .unwrap();
        executor.result_committed(cancel).await.unwrap();
        sessions.processes().shutdown().await.unwrap();
    }
}

#[test]
fn resource_picker_rejects_an_existing_conversations_unbound_host() {
    let (_root, store, record) = fixture("paper");
    let id = record.input.context[0].resource_id.clone();
    let request = crate::agent::resource_catalog::Request {
        conversation_id: record.conversation_id,
        kind: "paper".into(),
        resource_id: id,
    };
    assert!(
        crate::agent::resource_catalog::list(&store, &request)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        crate::agent::resource_catalog::list(
            &store,
            &crate::agent::resource_catalog::Request {
                resource_id: Uuid::new_v4().to_string(),
                ..request
            }
        ),
        Err(crate::agent::resource_catalog::Error::Unbound)
    ));
}
