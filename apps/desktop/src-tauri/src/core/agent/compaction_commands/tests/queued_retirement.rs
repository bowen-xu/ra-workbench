//! The original queued claim fences credential preparation and real admission.
use super::*;
use crate::{
    agent::{
        repository::input_queue,
        writing_submission::{WritingSubmissionReceipt, WritingSubmissionService},
    },
    data_interface::{CancelPreviewIntent, Database, SharedDatabase},
};
use research_agent_core::context::compaction_hooks::{
    CompactionHookFuture, CompactionHookNotification, CompactionHookOutcome, CompactionHookRequest,
    CompactionHooks,
};
use research_agent_core::input_queue::{InputQueue, QueueEdit, QueuedInput};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct ObservedHooks(AtomicUsize);
impl CompactionHooks for ObservedHooks {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { CompactionHookOutcome::Continue })
    }
}

struct QueuedFixture {
    original: Fixture,
    db: SharedDatabase,
    cancellation: CancelPreviewIntent,
}
impl QueuedFixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        let request = Uuid::new_v4();
        let hash = format!("{:x}", Sha256::digest(b"Retained queued source"));
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic queued compact')",
            [conversation.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')", []).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')", [manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)", params![conversation.to_string(), manuscript.to_string()]).unwrap();
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',22,?2,'now')", params![manuscript.to_string(), hash]).unwrap();
        sql.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('deepseek',1,'',60000,'now') on conflict(provider_id) do update set enabled=1", []).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
        let source = store
            .manuscript_directory()
            .join(manuscript.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Retained queued source").unwrap();
        input_queue::apply_checked(
            &mut db.lock().unwrap(),
            conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input,
                text: "/compact".into(),
                user_input: vec![],
                origin: None,
            }),
            |_| Ok(()),
        )
        .unwrap();
        let credentials =
            JsonCredentialStore::from_path(root.path().join("synthetic-credentials.json"));
        credentials
            .set("deepseek", "synthetic-compaction-fixture")
            .unwrap();
        let models = AgentModelResolver::from_app_store(&store, credentials);
        let service = WritingSubmissionService::from_app_store(&store);
        crate::agent::writing_queued_compaction::prepare(&service, &models,
            serde_json::from_value(json!({"inputId":input,"expectedQueueRevision":1,"previousRequestId":null,"expectedRevision":0,
                "submission":{"conversationId":conversation,"requestId":request,"text":"/compact","userInput":[],
                    "context":{"manuscriptId":manuscript,"files":[{"relativePath":"main.tex","expectedDiskSha256":hash}]},
                    "model":{"providerId":"deepseek","modelId":"deepseek-v4-pro"}}})).unwrap()).await.unwrap();
        let record = service
            .restore_record(WritingSubmissionReceipt {
                conversation_id: conversation,
                request_id: request,
                manuscript_id: manuscript,
            })
            .await
            .unwrap();
        let request = StartRequest {
            conversation_id: conversation,
            request_id: request,
            context_snapshot_id: record.context_snapshot_id,
            expected_revision: 0,
            model: record.input.model.clone(),
            host: Host::Manuscript {
                manuscript_id: manuscript,
            },
        };
        let preview_request_id = request.request_id;
        let expected_queue_revision =
            input_queue::read_checked(&mut db.lock().unwrap(), conversation, |_| Ok(()))
                .unwrap()
                .revision();
        Self {
            original: Fixture {
                _root: root,
                store,
                sessions,
                record,
                request,
            },
            db,
            cancellation: CancelPreviewIntent {
                conversation_id: conversation,
                manuscript_id: manuscript,
                request_id: Uuid::new_v4(),
                expected_queue_revision,
                input_id: input,
                preview_request_id,
            },
        }
    }
    fn queue(&self) -> InputQueue {
        input_queue::read_checked(
            &mut self.db.lock().unwrap(),
            self.cancellation.conversation_id,
            |_| Ok(()),
        )
        .unwrap()
    }
    async fn history_count(&self) -> u64 {
        self.original
            .sessions
            .open(ConversationId::from_uuid(self.cancellation.conversation_id))
            .await
            .unwrap()
            .seq()
    }
}

#[tokio::test]
async fn retired_claim_rejects_before_transport_and_hook_preparation() {
    let f = QueuedFixture::new().await;
    let before = f.queue();
    f.db.cancel_queue_preview(&f.cancellation).unwrap();
    let transport = transport(&f.original, false);
    let hooks = Arc::new(ObservedHooks::default());
    let factory_calls = AtomicUsize::new(0);
    let result = start_prepared(
        &f.original.store,
        f.original.sessions.clone(),
        f.original.request.clone(),
        |_, _| async {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                transport.clone() as Arc<dyn ModelTransport>,
                Some(hooks.clone() as Arc<dyn CompactionHooks>),
                empty_tools(),
            ))
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    assert_eq!(hooks.0.load(Ordering::SeqCst), 0);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(f.history_count().await, 0);
    assert_eq!(f.queue().inputs(), before.inputs());
    assert!(f.queue().paused());
    assert_eq!(f.queue().preparing(), None);
}

#[tokio::test]
async fn retirement_during_transport_preparation_blocks_task_model_and_hooks() {
    let f = QueuedFixture::new().await;
    let transport = transport(&f.original, false);
    let hooks = Arc::new(ObservedHooks::default());
    let (entered, entered_rx) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let store = f.original.store.clone();
    let sessions = f.original.sessions.clone();
    let request = f.original.request.clone();
    let held_transport = transport.clone();
    let held_hooks = hooks.clone();
    let starting = tokio::spawn(async move {
        start_prepared(&store, sessions, request, |_, _| async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            Ok((
                held_transport as Arc<dyn ModelTransport>,
                Some(held_hooks as Arc<dyn CompactionHooks>),
                empty_tools(),
            ))
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered_rx)
        .await
        .unwrap()
        .unwrap();
    f.db.cancel_queue_preview(&f.cancellation).unwrap();
    release.send(()).unwrap();
    assert!(starting.await.unwrap().is_err());
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(hooks.0.load(Ordering::SeqCst), 0);
    assert_eq!(f.history_count().await, 0);
    assert!(f.queue().paused());
    assert_eq!(f.queue().preparing(), None);
}

#[tokio::test]
async fn admitted_start_preserves_claim_for_original_stop_and_settlement() {
    let f = QueuedFixture::new().await;
    let transport = transport(&f.original, true);
    let hooks = Arc::new(ObservedHooks::default());
    let admitted = start_prepared(
        &f.original.store,
        f.original.sessions.clone(),
        f.original.request.clone(),
        |_, _| async {
            Ok((
                transport.clone() as Arc<dyn ModelTransport>,
                Some(hooks.clone() as Arc<dyn CompactionHooks>),
                empty_tools(),
            ))
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.reached.notified(),
    )
    .await
    .unwrap();
    assert!(hooks.0.load(Ordering::SeqCst) > 0);
    let before = f.queue();
    assert!(f.db.cancel_queue_preview(&f.cancellation).is_err());
    assert_eq!(f.queue(), before);
    assert!(
        f.original
            .sessions
            .tasks()
            .cancel_turn(
                ConversationId::from_uuid(f.cancellation.conversation_id),
                f.original.record.turn_id
            )
            .unwrap()
    );
    assert_eq!(wait(admitted).await, TaskCompletion::Canceled);
    let current = f.queue();
    let settled =
        f.db.settle_queued_compaction(
            &crate::agent::repository::input_queue::queued_compaction::SettleIntent {
                operation_id: Uuid::new_v4(),
                conversation_id: f.cancellation.conversation_id,
                manuscript_id: f.cancellation.manuscript_id,
                input_id: f.cancellation.input_id,
                request_id: f.cancellation.preview_request_id,
                expected_queue_revision: current.revision(),
                claim_revision: 1,
            },
        )
        .unwrap();
    assert_eq!(settled.outcome, TurnOutcome::Canceled);
    assert_eq!(f.queue().inputs(), before.inputs());
    assert!(f.queue().paused());
    assert_eq!(f.queue().preparing(), None);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
