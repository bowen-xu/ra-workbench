//! Preserve Core authorization and the complete submitted-source prefix around Native waits.
use super::*;
use crate::{
    agent::{reading_sources, repository::submissions},
    ai::paper_context::agent_preview,
};
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
use research_workbench_native_sdk::catalog::CatalogOutput;
use uuid::Uuid;

fn prepare(f: &Fixture) -> (reading_sources::Request, Uuid, Vec<CatalogOutput>) {
    let (snapshot, outputs) = super::paper_preview::prepare(f);
    let mut db = f.store.connection().unwrap();
    let package = f.service.catalog.discover().unwrap().packages.remove(0);
    let preview = crate::runtime_plugins::literature_context::test_fixture::expected_preview(
        &f.service.catalog,
        &package,
        &db,
        &snapshot,
    );
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "Original paper question".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: agent_preview::submission_context_from_preview(preview).unwrap(),
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    };
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Paper source restoration')",
        [conversation.to_string()],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','paper')", [conversation.to_string()]).unwrap();
    let tx = db.transaction().unwrap();
    let record = submissions::insert_in(&tx, conversation, Uuid::new_v4(), input).unwrap();
    tx.execute("insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,'paper',?4,?5)", rusqlite::params![conversation.to_string(), record.request_id.to_string(), record.turn_id.to_string(), snapshot.snapshot_id, snapshot.expected_context_hash]).unwrap();
    tx.commit().unwrap();
    let request = serde_json::from_value(
        json!({"conversationId":conversation,"paperId":"paper","turnId":record.turn_id}),
    )
    .unwrap();
    (request, conversation, outputs)
}
fn spawn(
    f: &Fixture,
    request: reading_sources::Request,
) -> tokio::task::JoinHandle<Result<reading_sources::Response, Value>> {
    let store = f.store.clone();
    tokio::spawn(async move { reading_sources::read(&store, request).await })
}
#[tokio::test]
async fn raw_submission_change_after_wait_rejects_before_the_next_native_stage() {
    let mut f = Fixture::new();
    let (request, conversation, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_submissions set input_json=input_json || ' ' where conversation_id=?1",
            [conversation.to_string()],
        )
        .unwrap();
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error, json!("conflict"));
    assert!(!f.lifecycle().contains("call 1"));
    assert_eq!(f.counts(), (1, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn revoked_binding_after_wait_keeps_the_original_core_rejection_priority() {
    let mut f = Fixture::new();
    let (request, conversation, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [conversation.to_string()],
        )
        .unwrap();
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error, json!("unbound"));
    assert!(!f.lifecycle().contains("call 1"));
    assert_eq!(f.counts(), (1, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn source_sql_failure_precedes_host_exchange_and_retains_safe_sqlite_diagnostics() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let mut f = Fixture::new();
    let (request, _, outputs) = prepare(&f);
    f.configure(&outputs, None, false);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "agent_conversation_bindings",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = reading_sources::read(&f.store, request).await.unwrap_err();
    assert_eq!(error["code"], "unavailable", "{error}");
    assert_eq!(
        error["nativeFailure"]["databaseError"]["sqliteExtendedCode"],
        23
    );
    assert!(!f.lifecycle().contains("call 0"));
    assert_eq!(f.counts(), (1, 0));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_source_waiter_keeps_the_owner_until_read_completion() {
    let mut f = Fixture::new();
    let (request, _, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(f.store.connection().unwrap().is_autocommit());
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            if f.service.doi_operations.try_write().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(f.lifecycle().contains("call 2"));
    assert_eq!(f.counts(), (1, 0));
    f.service.shutdown().await.unwrap();
}
