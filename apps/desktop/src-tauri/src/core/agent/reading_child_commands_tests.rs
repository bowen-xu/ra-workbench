use super::*;
use crate::{ai::JsonCredentialStore, data_interface::Database};
use research_agent_core::{
    protocol::ModelIdentity,
    submission::{SubmissionInput, SubmissionStore},
};

#[tokio::test]
async fn failed_paper_preparation_and_creation_replay_preserve_child() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = rusqlite::Connection::open(root.path().join("data.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let parent = Uuid::new_v4();
    sql.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','fixture','now','now')", []).unwrap();
    sql.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('snapshot','paper',?1,1,'now')", ["a".repeat(64)]).unwrap();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'parent')",
        [parent.to_string()],
    )
    .unwrap();
    sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','paper')", [parent.to_string()]).unwrap();
    let database = Database::open(sql, &root.path().join("log")).unwrap();
    let store = AppStore::for_test_database(database, root.path().join("objects"));
    let models = AgentModelResolver::from_app_store(
        &store,
        JsonCredentialStore::from_path(root.path().join("credentials.json")),
    );
    let catalog = SqliteConversationCatalog::from_app_store(&store);
    let submissions = SqliteSubmissionStore::for_app(&store);
    let record = submissions
        .put(
            parent,
            Uuid::new_v4(),
            SubmissionInput {
                remote_compaction_v2: None,
                text: "question".into(),
                user_input: vec![],
                skill_instructions: vec![],
                model: ModelIdentity {
                    provider_id: "missing-provider".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
            },
        )
        .await
        .unwrap();
    store.connection().unwrap().execute("insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,'paper','snapshot',?4)", rusqlite::params![parent.to_string(), record.request_id.to_string(), record.turn_id.to_string(), "a".repeat(64)]).unwrap();
    let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let admission = sessions
        .start_submission(
            &submissions,
            ConversationId::from_uuid(parent),
            record.request_id,
            move |_, _, _| async move {
                let _ = wait.await;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("parent must start")
    };
    let child = Uuid::new_v4();
    let request = || Request {
        parent_conversation_id: parent,
        parent_turn_id: record.turn_id,
        paper_id: "paper".into(),
        paper_context_snapshot_id: "snapshot".into(),
        expected_context_hash: Some("a".repeat(64)),
        prompt: "research".into(),
        approved_servers: HashSet::new(),
    };
    let first = spawn_paper_child_with_id(
        store.clone(),
        catalog.clone(),
        models.clone(),
        sessions.clone(),
        request(),
        child,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(first, Error::ChildPreparationUnconfirmed { child_id, reason: ChildPreparationReason::Preparation(reading_submission::Error::ModelUnavailable), .. } if child_id == child)
    );
    let second = spawn_paper_child_with_id(
        store.clone(),
        catalog,
        models,
        sessions.clone(),
        request(),
        child,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(second, Error::ChildPreparationUnconfirmed { child_id, reason: ChildPreparationReason::CreationReplayed, .. } if child_id == child)
    );
    let db = store.connection().unwrap();
    let (archived, submissions, cleanup): (bool, u64, u64) = db.query_row("select (select archived from agent_conversations where id=?1), (select count(*) from agent_submissions where conversation_id=?1), (select count(*) from agent_child_cleanup_intents where child_id=?1)", [child.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!((archived, submissions, cleanup), (false, 0, 0));
    drop(db);
    release.send(()).unwrap();
    let _ = owned.task.wait().await;
}
