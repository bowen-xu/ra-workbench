//! Original paper SQL settlement and replay identity around bounded Native waits.
use super::*;
use crate::{agent::reading_submission, data_interface::PaperPrepareIntent};
use research_agent_core::protocol::ModelIdentity;
use research_workbench_native_sdk::catalog::CatalogOutput;
use uuid::Uuid;

pub(super) fn prepare(f: &Fixture) -> (PaperPrepareIntent, Vec<CatalogOutput>) {
    let (snapshot, outputs) = super::paper_preview::prepare(f);
    let conversation = Uuid::new_v4();
    let db = f.store.connection().unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Paper proof')",
        [conversation.to_string()],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','paper')", [conversation.to_string()]).unwrap();
    let request = serde_json::from_value(json!({
        "conversationId":conversation,"requestId":Uuid::new_v4(),"paperId":"paper",
        "paperContextSnapshotId":snapshot.snapshot_id,"expectedContextHash":snapshot.expected_context_hash,
        "text":"Read the frozen paper","userInput":[],"retrieval":null,"model":null
    })).unwrap();
    (
        PaperPrepareIntent {
            request,
            expected_epoch: None,
        },
        outputs,
    )
}
fn model() -> (ModelIdentity, Option<String>, Option<String>) {
    (
        ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: None,
        },
        None,
        None,
    )
}
pub(super) fn counts(f: &Fixture) -> (i64, i64, i64) {
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit(), "no transaction may cross a Native wait");
    db.query_row("select (select count(*) from agent_submissions where conversation_id in (select conversation_id from agent_conversation_bindings where resource_kind='paper')),(select count(*) from agent_paper_submission_scopes),(select count(*) from data_sql_operation_receipts where kind='agent.paper.submission.prepare')",[], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}
pub(super) fn spawn(
    f: &Fixture,
    intent: PaperPrepareIntent,
) -> tokio::task::JoinHandle<
    Result<
        (
            research_agent_core::submission::SubmissionRecord,
            crate::data_interface::SqlImmediateReceipt,
        ),
        reading_submission::Error,
    >,
> {
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move {
        service
            .prepare_paper_submission(store, key, intent, model())
            .await
    })
}
#[tokio::test]
async fn paused_preparation_rolls_back_and_rechecks_revoked_binding_before_the_next_stage() {
    let mut f = Fixture::new();
    let (intent, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let conversation = intent.request.conversation_id;
    let caller = spawn(&f, intent);
    assert!(f.entered(0).await);
    assert_eq!(counts(&f), (0, 0, 0));
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
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["reason"]["code"], "DATA_SQL_INVALID");
    assert_eq!(
        value["reason"]["reason"]["safeCause"],
        "paper_preparation_unbound"
    );
    assert!(!f.lifecycle().contains("call 1"));
    assert_eq!(counts(&f), (0, 0, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn detached_receiver_does_not_cancel_original_core_preparation() {
    let mut f = Fixture::new();
    let (intent, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, intent);
    assert!(f.entered(0).await);
    assert_eq!(counts(&f), (0, 0, 0));
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    f.release();
    tokio::time::timeout(WAIT, async {
        while counts(&f) != (1, 1, 1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.service.shutdown().await.unwrap();
    assert!(f.lifecycle().ends_with("shutdown\n"));
}
#[tokio::test]
async fn replay_rechecks_raw_submission_bytes_after_wait_even_when_the_record_is_equal() {
    let mut f = Fixture::new();
    let (intent, mut outputs) = prepare(&f);
    outputs.extend(outputs.clone());
    f.configure(&outputs, Some(3), false);
    let (record, _) = spawn(&f, intent.clone()).await.unwrap().unwrap();
    assert_eq!(counts(&f), (1, 1, 1));
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    let caller =
        tokio::spawn(async move { service.paper_prepare_status(store, key, intent).await });
    assert!(f.entered(3).await);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_submissions set input_json=input_json || ' ' where conversation_id=?1",
            [record.conversation_id.to_string()],
        )
        .unwrap();
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .err()
        .expect("changed replay prefix");
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert_eq!(
        value["reason"]["reason"]["safeCause"],
        "paper_preparation_conflict"
    );
    assert!(!f.lifecycle().contains("call 4"));
    assert_eq!(counts(&f), (1, 1, 1));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_pause_rollback_keeps_original_recovery_fence_and_never_starts_native() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let mut f = Fixture::new();
    let (intent, outputs) = prepare(&f);
    let operation = intent.request.request_id;
    f.configure(&outputs, None, false);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = spawn(&f, intent).await.unwrap().unwrap_err();
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["reason"]["operationId"],
        operation.to_string()
    );
    assert_eq!(value["reason"]["reason"]["phase"], "rollback");
    assert_eq!(
        value["reason"]["reason"]["safeCause"],
        "sql_rollback_failed"
    );
    assert!(f.store.shared_agent_connection().lock().is_err());
    assert!(!f.lifecycle().contains("initialize"));
    f.service.shutdown().await.unwrap();
}
