use super::*;
#[path = "tests/context.rs"]
mod context;
use crate::{
    agent::writing_context::{WritingContextFile, WritingContextRequest},
    agent::{
        repository::input_queue,
        writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
    },
    data_interface::{SharedDatabase, journal::JournalCommand},
    storage::AppStore,
};
use research_agent_core::{
    conversation::{CatalogError, ConversationId},
    input_queue::{QueueEdit, QueuedInput},
    protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome},
};
use research_data_runtime::{WriteMode, WriteOptions};
use rusqlite::{Connection, params};
use serde_json::json;

#[path = "tests/failures.rs"]
mod failures;
#[path = "tests/reconciliation.rs"]
mod reconciliation;
#[path = "tests/retirement.rs"]
mod retirement;
#[path = "tests/retirement_races.rs"]
mod retirement_races;
#[path = "tests/retirement_read.rs"]
mod retirement_read;

struct Fixture {
    root: tempfile::TempDir,
    db: SharedDatabase,
    store: AppStore,
    conversation: Uuid,
    manuscript: Uuid,
    input: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic compact')",
            [conversation.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![conversation.to_string(),manuscript.to_string()]).unwrap();
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',17,?2,'now')",params![manuscript.to_string(),format!("{:x}",Sha256::digest(b"Frozen manuscript"))]).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        let source = store
            .manuscript_directory()
            .join(manuscript.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
        let f = Self {
            root,
            db,
            store,
            conversation,
            manuscript,
            input,
        };
        f.edit(QueueEdit::Add(QueuedInput {
            id: input,
            text: "/compact".into(),
            user_input: vec![],
            origin: None,
        }));
        f
    }
    fn intent(&self, request: Uuid, previous: Option<Uuid>) -> PrepareIntent {
        let q = self.q();
        let w = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [self.conversation.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        PrepareIntent {
            conversation_id: self.conversation,
            manuscript_id: self.manuscript,
            input_id: self.input,
            request_id: request,
            expected_queue_revision: q.revision(),
            expected_revision: w,
            previous_request_id: previous,
            request_json: json!({"inputId":self.input,"expectedQueueRevision":q.revision(),"previousRequestId":previous,"submission":{"conversationId":self.conversation,"requestId":request,"text":"/compact","userInput":[],"context":{"manuscriptId":self.manuscript,"files":[{"relativePath":"main.tex","expectedDiskSha256":format!("{:x}",Sha256::digest(b"Frozen manuscript")),"buffer":null,"selection":null}]},"resources":[],"retrieval":null,"model":null},"expectedRevision":w}),
        }
    }
    fn source(&self, command: &PrepareIntent) -> PrepareSource {
        PrepareSource {
            store: self.store.clone(),
            request: WritingSubmissionRequest {
                conversation_id: self.conversation,
                request_id: command.request_id,
                text: "/compact".into(),
                user_input: vec![],
                context: WritingContextRequest {
                    manuscript_id: self.manuscript,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
                        buffer: None,
                        selection: None,
                    }],
                },
                resources: vec![],
                retrieval: None,
                model: None,
                session_config: None,
                turn_config: None,
            },
            model: FrozenWritingModel {
                identity: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "synthetic".into(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
            },
        }
    }
    fn prepare(&self, command: &PrepareIntent) -> QueuedCompactionState {
        self.db
            .prepare_queued_compaction(command, self.source(command))
            .unwrap()
    }
    fn q(&self) -> research_agent_core::input_queue::InputQueue {
        input_queue::read_checked(&mut self.db.lock().unwrap(), self.conversation, |_| Ok(()))
            .unwrap()
    }
    fn edit(&self, edit: QueueEdit) {
        let revision = self.q().revision();
        input_queue::apply_checked(
            &mut self.db.lock().unwrap(),
            self.conversation,
            Uuid::new_v4(),
            revision,
            edit,
            |_| Ok(()),
        )
        .unwrap();
    }
    fn read(&self, request: Option<Uuid>) -> Option<QueuedCompactionRead> {
        self.db
            .read_queued_compaction(&ReadIntent {
                conversation_id: self.conversation,
                manuscript_id: self.manuscript,
                input_id: self.input,
                request_id: request,
            })
            .unwrap()
    }
    fn settle(&self, command: &PrepareIntent) -> SettleIntent {
        SettleIntent {
            operation_id: Uuid::new_v4(),
            conversation_id: self.conversation,
            manuscript_id: self.manuscript,
            input_id: self.input,
            request_id: command.request_id,
            expected_queue_revision: self.q().revision(),
            claim_revision: command.expected_queue_revision,
        }
    }
    fn events(
        &self,
        record: &research_agent_core::submission::SubmissionRecord,
        payloads: Vec<EventPayload>,
    ) {
        let count: u64 = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [self.conversation.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(n, payload)| {
                AgentEvent::new(
                    ConversationId::from_uuid(self.conversation),
                    count + n as u64 + 1,
                    0,
                    record.request_id,
                    payload,
                )
                .unwrap()
            })
            .collect();
        self.db
            .write(
                JournalCommand::new(
                    ConversationId::from_uuid(self.conversation),
                    Uuid::new_v4(),
                    count,
                    events,
                    None,
                    1_790_000_000_000,
                )
                .unwrap(),
                WriteOptions {
                    mode: WriteMode::Immediate,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    fn terminal(&self, state: &QueuedCompactionState, outcome: TurnOutcome) {
        self.events(
            &state.record,
            vec![
                EventPayload::TurnStarted {
                    task_kind: TaskKind::Compact,
                    turn_id: state.record.turn_id,
                    context_snapshot_id: state.record.context_snapshot_id,
                    model: state.record.input.model.clone(),
                },
                EventPayload::TurnFinished {
                    turn_id: state.record.turn_id,
                    outcome,
                },
            ],
        );
    }
    fn reopen(self) -> Self {
        let Self {
            root,
            db,
            store,
            conversation,
            manuscript,
            input,
        } = self;
        drop(store);
        drop(db);
        let mut sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        Self {
            root,
            db,
            store,
            conversation,
            manuscript,
            input,
        }
    }
}

#[test]
fn prepare_is_atomic_keeps_i_and_replay_skips_fresh_sources_with_purpose_aware_recovery() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let first = f.prepare(&command);
    assert_eq!(first.queue.revision(), command.expected_queue_revision + 1);
    assert_eq!(first.queue.preparing(), Some(f.input));
    assert_eq!(first.queue.inputs().len(), 1);
    assert_eq!(first.claim_revision, command.expected_queue_revision);
    assert!(
        input_queue::read_pending_preview_checked(
            &mut f.db.lock().unwrap(),
            f.conversation,
            |_| Ok(())
        )
        .unwrap()
        .is_none()
    );
    assert!(
        matches!(f.read(None),Some(QueuedCompactionRead::Compact(state)) if state.current&&state.record==first.record)
    );
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    assert_eq!(
        f.db.replay_queued_compaction_prepare(&command)
            .unwrap()
            .unwrap()
            .record,
        first.record
    );
    assert_eq!(f.prepare(&command).record, first.record);
    let mut changed = command.clone();
    changed.request_json["submission"]["text"] = json!("Changed");
    assert!(f.db.replay_queued_compaction_prepare(&changed).is_err());
    f.db.shutdown().unwrap();
    assert!(f.db.replay_queued_compaction_prepare(&command).is_err());
}

#[test]
fn no_terminal_cannot_settle_or_consume_and_completed_task_removes_only_its_i() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&command);
    assert!(matches!(
        f.db.settle_queued_compaction(&f.settle(&command)),
        Err(SqlImmediateFailure::Domain(CatalogError::Busy))
    ));
    let tail = Uuid::new_v4();
    f.edit(QueueEdit::Add(QueuedInput {
        id: tail,
        text: "Later input".into(),
        user_input: vec![],
        origin: None,
    }));
    f.terminal(&state, TurnOutcome::Completed);
    let settle = f.settle(&command);
    let first = f.db.settle_queued_compaction(&settle).unwrap();
    assert_eq!(first.queue.inputs()[0].id, tail);
    assert_eq!(first.queue.inputs().len(), 1);
    assert_eq!(first.queue.preparing(), None);
    assert_eq!(first.outcome, TurnOutcome::Completed);
    assert_eq!(
        f.db.settle_queued_compaction(&settle).unwrap().queue,
        first.queue
    );
    assert!(
        matches!(f.read(Some(command.request_id)),Some(QueuedCompactionRead::Compact(state)) if !state.current&&state.outcome==Some(TurnOutcome::Completed))
    );
}

#[test]
fn failed_canceled_and_interrupted_release_pause_and_old_settlement_cannot_touch_new_claim() {
    for outcome in [
        TurnOutcome::Failed,
        TurnOutcome::Canceled,
        TurnOutcome::Interrupted,
    ] {
        let f = Fixture::new();
        let command = f.intent(Uuid::new_v4(), None);
        let state = f.prepare(&command);
        let original_record = state.record.clone();
        f.terminal(&state, outcome);
        let settle = f.settle(&command);
        let released = f.db.settle_queued_compaction(&settle).unwrap();
        assert!(released.queue.paused());
        assert_eq!(released.queue.preparing(), None);
        assert_eq!(released.queue.inputs()[0].id, f.input);
        f.edit(QueueEdit::Resume);
        let newer = f.intent(Uuid::new_v4(), Some(command.request_id));
        let active = f.prepare(&newer);
        assert!(
            matches!(f.read(Some(command.request_id)),Some(QueuedCompactionRead::Compact(state)) if !state.current&&state.record==original_record)
        );
        let prepare_replay = f.prepare(&command);
        assert_eq!(prepare_replay.record, original_record);
        assert_eq!(prepare_replay.queue, state.queue);
        assert!(prepare_replay.current);
        assert_eq!(prepare_replay.outcome, None);
        let replay = f.db.settle_queued_compaction(&settle).unwrap();
        assert_eq!(replay.outcome, outcome);
        assert_eq!(f.q().preparing(), Some(f.input));
        assert_eq!(f.q().revision(), active.queue.revision());
        let stale = SettleIntent {
            operation_id: Uuid::new_v4(),
            expected_queue_revision: f.q().revision(),
            ..settle
        };
        assert!(f.db.settle_queued_compaction(&stale).is_err());
        assert_eq!(f.q(), active.queue);
    }
}

#[test]
fn cross_scope_watermark_revision_and_non_head_claims_fail_without_a_preparation() {
    let f = Fixture::new();
    let original = f.intent(Uuid::new_v4(), None);
    for variant in 0..5 {
        let mut command = original.clone();
        match variant {
            0 => command.manuscript_id = Uuid::new_v4(),
            1 => command.input_id = Uuid::new_v4(),
            2 => command.expected_revision += 1,
            3 => command.expected_queue_revision += 1,
            _ => command.previous_request_id = Some(Uuid::new_v4()),
        };
        assert!(
            f.db.prepare_queued_compaction(&command, f.source(&command))
                .is_err()
        );
        assert_eq!(f.q().preparing(), None);
    }
    let later = Uuid::new_v4();
    f.edit(QueueEdit::Add(QueuedInput {
        id: later,
        text: "/compact".into(),
        user_input: vec![],
        origin: None,
    }));
    let mut command = f.intent(Uuid::new_v4(), None);
    command.input_id = later;
    assert!(
        f.db.prepare_queued_compaction(&command, f.source(&command))
            .is_err()
    );
    assert_eq!(f.q().inputs().len(), 2);
}

#[test]
fn startup_pause_keeps_original_claim_explicitly_startable_without_resuming_following_inputs() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    f.prepare(&command);
    input_queue::pause_on_startup(&mut f.db.lock().unwrap()).unwrap();
    let before = f.q();
    assert!(before.paused());
    queue::require_startable_in(&f.db.lock().unwrap(), f.conversation, command.request_id).unwrap();
    assert_eq!(f.q(), before);
    let f = f.reopen();
    assert!(
        matches!(f.read(Some(command.request_id)),Some(QueuedCompactionRead::Compact(state)) if state.current&&state.queue.paused())
    );
}

#[test]
fn discovery_recovers_released_attempt_and_stops_after_completed_input_removal() {
    let f = Fixture::new();
    let first = f.intent(Uuid::new_v4(), None);
    let prepared = f.prepare(&first);
    f.terminal(&prepared, TurnOutcome::Failed);
    f.db.settle_queued_compaction(&f.settle(&first)).unwrap();
    let Some(QueuedCompactionRead::Compact(recovered)) = f.read(None) else {
        panic!("retained input must discover its latest attempt")
    };
    assert_eq!(recovered.record.request_id, first.request_id);
    assert!(!recovered.current);
    assert_eq!(recovered.outcome, Some(TurnOutcome::Failed));
    assert_eq!(recovered.queue, prepared.queue);
    assert!(f.read(Some(Uuid::new_v4())).is_none());
    f.edit(QueueEdit::Resume);
    let second = f.intent(Uuid::new_v4(), Some(recovered.record.request_id));
    let retried = f.prepare(&second);
    assert!(
        matches!(f.read(None),Some(QueuedCompactionRead::Compact(state)) if state.current&&state.record.request_id==second.request_id)
    );
    f.terminal(&retried, TurnOutcome::Completed);
    f.db.settle_queued_compaction(&f.settle(&second)).unwrap();
    assert!(f.read(None).is_none());
    assert!(
        matches!(f.read(Some(first.request_id)),Some(QueuedCompactionRead::Compact(state)) if !state.current&&state.outcome==Some(TurnOutcome::Failed))
    );
    assert!(
        matches!(f.read(Some(second.request_id)),Some(QueuedCompactionRead::Compact(state)) if !state.current&&state.outcome==Some(TurnOutcome::Completed))
    );
}
