use super::*;
#[path = "tests/configured_hook.rs"]
mod configured_hook;
#[path = "tests/hook_factory.rs"]
mod hook_factory;
#[path = "tests/post_install_failure.rs"]
mod post_install_failure;
#[path = "tests/queued_retirement.rs"]
mod queued_retirement;
#[path = "tests/tool_catalog.rs"]
mod tool_catalog;
#[path = "tests/worker_retention.rs"]
mod worker_retention;
use crate::{
    agent::{note_submission, tasks::TaskCompletion},
    ai::JsonCredentialStore,
};
use research_agent_core::{
    model::*,
    protocol::{EventPayload, TaskKind, TurnOutcome},
    submission::SubmissionStore,
};
use rusqlite::{Connection, params};
use std::{collections::VecDeque, sync::Mutex};

async fn start_prepared<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<PreparedCompaction, Error>>,
{
    start_prepared_retaining(
        store,
        sessions,
        request,
        WorkerRetention::default(),
        factory,
    )
    .await
}

struct Fixture {
    _root: tempfile::TempDir,
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    record: SubmissionRecord,
    request: StartRequest,
}
async fn fixture(text: &str) -> Fixture {
    fixture_model(text, "deepseek-v4-pro").await
}
async fn fixture_model(text: &str, model_id: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = AppStore::for_test(db, root.path().canonicalize().unwrap().join("objects"));
    let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let conversation_id = Uuid::new_v4();
    sessions
        .create(ConversationId::from_uuid(conversation_id))
        .await
        .unwrap();
    let page_id = Uuid::new_v4();
    {
        let db = store.connection().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values (?1,'Synthetic note','now','now')", [page_id.to_string()]).unwrap();
        let content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Command-only source","styles":[]}]}).to_string();
        db.execute("insert into note_blocks(id,page_id,scope_kind,block_type,content_json,created_at,updated_at) values ('selected',?1,'page','paragraph',?2,'now','now')", params![page_id.to_string(),content]).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note',?2)", params![conversation_id.to_string(),page_id.to_string()]).unwrap();
        db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('deepseek',1,'',60000,'now') on conflict(provider_id) do update set enabled=1", []).unwrap();
    }
    let credentials =
        JsonCredentialStore::from_path(root.path().join("synthetic-credentials.json"));
    credentials
        .set("deepseek", "synthetic-compaction-fixture")
        .unwrap();
    let models = AgentModelResolver::from_app_store(&store, credentials);
    let record = note_submission::prepare(
        &store,
        &models,
        note_submission::PrepareRequest {
            conversation_id,
            request_id: Uuid::new_v4(),
            text: text.into(),
            user_input: vec![],
            context: crate::notes::agent_context::NoteContextRequest {
                page_id,
                expected_revision: 0,
                block_id: Some("selected".into()),
            },
            retrieval: None,
            model: Some(super::super::model::ModelSelection {
                provider_id: "deepseek".into(),
                model_id: model_id.into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .unwrap();
    let request = StartRequest {
        conversation_id,
        request_id: record.request_id,
        context_snapshot_id: record.context_snapshot_id,
        expected_revision: 0,
        model: record.input.model.clone(),
        host: Host::Note { page_id },
    };
    Fixture {
        _root: root,
        store,
        sessions,
        record,
        request,
    }
}
struct Transport {
    model: ModelIdentity,
    requests: Mutex<Vec<ModelRequest>>,
    pending: bool,
    reached: tokio::sync::Notify,
}
struct Stream(VecDeque<ModelStreamEvent>);
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            self.reached.notify_one();
            if self.pending {
                std::future::pending::<()>().await;
            }
            Ok(Box::new(Stream(VecDeque::from([
                ModelStreamEvent::Started {
                    response_id: "compact-fixture".into(),
                    actual_model: self.model.clone(),
                },
                ModelStreamEvent::TextDelta("Retain research decisions and source anchors.".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ]))) as Box<dyn ModelStream>)
        })
    }
}
fn transport(f: &Fixture, pending: bool) -> Arc<Transport> {
    Arc::new(Transport {
        model: f.record.input.model.clone(),
        requests: Mutex::new(vec![]),
        pending,
        reached: tokio::sync::Notify::new(),
    })
}
fn empty_tools() -> OwnedToolRegistration {
    OwnedToolRegistration::new(
        vec![],
        Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
    )
}
async fn wait(admission: SubmissionAdmission) -> TaskCompletion {
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task expected")
    };
    owned.task.wait().await
}
async fn advance(f: &Fixture) {
    let store = SqliteSubmissionStore::new(f.store.shared_agent_connection());
    let mut input = f.record.input.clone();
    input.text = "Intervening turn".into();
    let record = store
        .put(f.record.conversation_id, Uuid::new_v4(), input)
        .await
        .unwrap();
    let service = f.sessions.clone();
    let admitted = f
        .sessions
        .start_submission(
            &store,
            ConversationId::from_uuid(record.conversation_id),
            record.request_id,
            move |record, _, _| async move {
                service
                    .commit_payload(
                        ConversationId::from_uuid(record.conversation_id),
                        record.request_id,
                        EventPayload::TurnFinished {
                            turn_id: record.turn_id,
                            outcome: TurnOutcome::Completed,
                        },
                    )
                    .await
                    .map_err(|_| research_agent_core::runner::RunError::EventSink)
            },
        )
        .await
        .unwrap();
    assert_eq!(wait(admitted).await, TaskCompletion::Completed);
}

#[tokio::test]
async fn confirmed_native_compact_installs_window_and_retry_skips_credentials() {
    let f = fixture("/compact").await;
    let transport = transport(&f, false);
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    assert_eq!(wait(result).await, TaskCompletion::Completed);
    let history = f
        .sessions
        .open(ConversationId::from_uuid(f.record.conversation_id))
        .await
        .unwrap();
    let turn = history.turn(f.record.turn_id).unwrap();
    assert_eq!(turn.task_kind, TaskKind::Compact);
    assert_eq!(turn.items.len(), 1);
    assert!(turn.items.values().all(|item| item.completed));
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    let window = f
        .sessions
        .active_context_checkpoint(f.record.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.source_watermark, 0);
    {
        let calls = transport.requests.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].tools.is_empty());
        assert_eq!(calls[0].messages.len(), 2);
        assert!(
            matches!(&calls[0].messages[0], ModelMessage::System(text) if text == super::super::host_instructions::NOTES)
        );
    }
    let retry = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("retry refreshed credentials") },
    )
    .await
    .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    // Ordinary admission cannot reuse a Compact receipt as a user submission.
    let store = SqliteSubmissionStore::new(f.store.shared_agent_connection());
    assert!(
        f.sessions
            .existing_submission(&store, &f.record)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn confirmation_host_and_command_checks_precede_transport_preparation() {
    let f = fixture("/compact").await;
    for case in 0..7 {
        let mut request = f.request.clone();
        match case {
            0 => request.context_snapshot_id = Uuid::new_v4(),
            1 => request.model.model_id.push_str("-other"),
            2 => {
                request.host = Host::Note {
                    page_id: Uuid::new_v4(),
                }
            }
            3 => {
                request.host = Host::Manuscript {
                    manuscript_id: Uuid::new_v4(),
                }
            }
            4 => {
                request.host = Host::Paper {
                    paper_id: "other".into(),
                    paper_context_snapshot_id: "other".into(),
                }
            }
            5 => {
                request.host = Host::ReadingNote {
                    reading_note_id: Uuid::new_v4(),
                }
            }
            _ => request.expected_revision = u64::MAX,
        }
        let result = start_checked(&f.store, f.sessions.clone(), request, |_, _| async {
            panic!("invalid confirmation prepared transport")
        })
        .await;
        assert!(result.is_err());
    }
    let ordinary = fixture("ordinary user input").await;
    assert!(matches!(
        start_checked(
            &ordinary.store,
            ordinary.sessions.clone(),
            ordinary.request.clone(),
            |_, _| async { panic!("ordinary input became command") }
        )
        .await,
        Err(Error::ConfirmationMismatch)
    ));
    assert_eq!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .seq(),
        0
    );
}

#[tokio::test]
async fn changed_confirmation_and_race_during_preparation_never_start_model() {
    let f = fixture("/compact").await;
    let transport = transport(&f, false);
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async {
            advance(&f).await;
            Ok(transport.clone() as Arc<dyn ModelTransport>)
        },
    )
    .await;
    assert!(matches!(result, Err(Error::HistoryChanged { expected: 0, actual }) if actual > 0));
    assert!(transport.requests.lock().unwrap().is_empty());
    assert!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .turn(f.record.turn_id)
            .is_none()
    );
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("stale confirmation refreshed credentials") },
    )
    .await;
    assert!(matches!(result, Err(Error::HistoryChanged { .. })));
}

#[tokio::test]
async fn binding_removed_during_preparation_rejects_before_task_admission() {
    let f = fixture("/compact").await;
    let transport = transport(&f, false);
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async {
            f.store
                .connection()
                .unwrap()
                .execute("delete from agent_conversation_bindings", [])
                .unwrap();
            Ok(transport.clone() as Arc<dyn ModelTransport>)
        },
    )
    .await;
    assert!(matches!(result, Err(Error::HostUnavailable)));
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .seq(),
        0
    );
}

#[tokio::test]
async fn shared_stop_cancels_native_compact_without_installing_or_restarting() {
    let f = fixture("/compact").await;
    let transport = transport(&f, true);
    let admitted = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.reached.notified(),
    )
    .await
    .unwrap();
    assert!(
        f.sessions
            .tasks()
            .cancel_turn(
                ConversationId::from_uuid(f.record.conversation_id),
                f.record.turn_id
            )
            .unwrap()
    );
    assert_eq!(wait(admitted).await, TaskCompletion::Canceled);
    assert!(
        f.sessions
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .is_none()
    );
    let retry = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("canceled receipt restarted") },
    )
    .await
    .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Canceled),
            ..
        }
    ));
}

#[path = "tests/contracts.rs"]
mod contracts;

#[tokio::test]
async fn frozen_unsupported_model_is_rejected_without_preparing_transport() {
    let f = fixture_model("/compact", "not-reviewed-model").await;
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("unsupported profile prepared transport") },
    )
    .await;
    assert!(matches!(result, Err(Error::UnsupportedModel)));
    assert_eq!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .seq(),
        0
    );
}
