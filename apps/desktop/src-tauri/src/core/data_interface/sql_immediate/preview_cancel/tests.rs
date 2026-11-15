use super::*;
use crate::{agent::repository::input_queue, data_interface::SharedDatabase};
use research_agent_core::{
    conversation::ConversationBinding,
    input_queue::{QueueEdit, QueuedInput},
    protocol::ModelIdentity,
    submission::{ContextSourceKind, ContextText, SubmissionInput, SubmissionRecord},
    user_input::UserInput,
};
use rusqlite::params;
use serde_json::json;

#[path = "tests/compatibility.rs"]
mod compatibility;
#[path = "tests/failures.rs"]
mod failures;

struct Fixture {
    root: tempfile::TempDir,
    db: SharedDatabase,
    intent: CancelPreviewIntent,
    record: SubmissionRecord,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("cancel.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        let preview = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic ordinary preview')",
            [conversation.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![conversation.to_string(),manuscript.to_string()]).unwrap();
        let text = "retained ordinary input🙂".to_owned();
        let user_input = vec![UserInput::Text {
            text: text.clone(),
            text_elements: vec![],
        }];
        input_queue::apply_checked(
            &mut sql,
            conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input,
                text: text.clone(),
                user_input: user_input.clone(),
                origin: Some(ConversationBinding {
                    kind: research_agent_core::conversation::BindingKind::Manuscript,
                    resource_id: manuscript.to_string(),
                }),
            }),
            |_| Ok(()),
        )
        .unwrap();
        let input_value = SubmissionInput {
            remote_compaction_v2: None,
            text,
            user_input,
            skill_instructions: vec![],
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "synthetic".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![ContextText {
                source_id: "synthetic-source".into(),
                kind: ContextSourceKind::Manuscript,
                resource_id: "main.tex".into(),
                revision: "synthetic-v1".into(),
                anchor: Some(json!({"manuscriptId":manuscript}).to_string()),
                text: "Frozen synthetic manuscript".into(),
            }],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        };
        let (record, _) = input_queue::prepare_preview_checked(
            &mut sql,
            conversation,
            preview,
            1,
            input,
            input_value,
            |_| Ok(()),
        )
        .unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        Self {
            root,
            db,
            record,
            intent: CancelPreviewIntent {
                conversation_id: conversation,
                manuscript_id: manuscript,
                request_id: Uuid::new_v4(),
                expected_queue_revision: 2,
                input_id: input,
                preview_request_id: preview,
            },
        }
    }
    fn queue(&self) -> InputQueue {
        input_queue::read_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            |_| Ok(()),
        )
        .unwrap()
    }
    fn observer(&self) -> Connection {
        Connection::open(self.root.path().join("cancel.sqlite3")).unwrap()
    }
    fn count(&self, sql: &Connection) -> (u64, u64) {
        let business = sql
            .query_row(
                "select count(*) from agent_input_queue_receipts where request_id=?1",
                [self.intent.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let typed = sql
            .query_row(
                "select count(*) from data_sql_operation_receipts where operation_id=?1",
                [self.intent.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        (business, typed)
    }
    fn legacy_cancel(&self) -> InputQueue {
        let c = &self.intent;
        input_queue::cancel_preview_checked(
            &mut self.db.lock().unwrap(),
            c.conversation_id,
            c.request_id,
            c.expected_queue_revision,
            c.input_id,
            c.preview_request_id,
            |_| Ok(()),
        )
        .unwrap()
    }
    fn next_preview(&self) -> (Uuid, InputQueue) {
        let c = &self.intent;
        let revision = self.queue().revision();
        let mut sql = self.db.lock().unwrap();
        let resumed = input_queue::apply_checked(
            &mut sql,
            c.conversation_id,
            Uuid::new_v4(),
            revision,
            QueueEdit::Resume,
            |_| Ok(()),
        )
        .unwrap();
        let newer = Uuid::new_v4();
        let (_, queue) = input_queue::prepare_preview_checked(
            &mut sql,
            c.conversation_id,
            newer,
            resumed.revision(),
            c.input_id,
            self.record.input.clone(),
            |_| Ok(()),
        )
        .unwrap();
        (newer, queue)
    }
    fn reopen(self) -> Self {
        let Self {
            root,
            db,
            intent,
            record,
        } = self;
        drop(db);
        let mut sql = Connection::open(root.path().join("cancel.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        Self {
            root,
            db,
            intent,
            record,
        }
    }
}

#[test]
fn typed_cancel_atomically_retains_input_and_status_and_late_replay_preserve_new_claim() {
    let f = Fixture::new();
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    let (queue, receipt) = f.db.cancel_queue_preview(&f.intent).unwrap();
    assert!(!receipt.replayed);
    assert_eq!(receipt.operation_id, f.intent.request_id);
    assert_eq!(queue.revision(), 3);
    assert!(queue.paused());
    assert_eq!(queue.preparing(), None);
    assert_eq!(queue.inputs()[0].id, f.intent.input_id);
    assert_eq!(f.count(&f.observer()), (1, 1));
    let (newer, current) = f.next_preview();
    let (old, replay) = f.db.cancel_queue_preview(&f.intent).unwrap();
    assert_eq!(old, queue);
    assert!(replay.replayed);
    assert_eq!(f.queue(), current);
    assert!(
        matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Committed{queue:old,receipt:proof} if old==queue&&proof.replayed&&proof.fingerprint==receipt.fingerprint)
    );
    assert_eq!(
        f.db.lock()
            .unwrap()
            .query_row(
                "select request_id from agent_input_queue_previews where input_id=?1",
                [f.intent.input_id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        newer.to_string()
    );
    let f = f.reopen();
    assert_eq!(f.db.cancel_queue_preview(&f.intent).unwrap().0, queue);
    assert_eq!(f.queue(), current);
}

#[test]
fn typed_identity_rejects_changed_c_m_i_preview_and_queue_revision() {
    let f = Fixture::new();
    f.db.cancel_queue_preview(&f.intent).unwrap();
    let before = f.queue();
    for variant in 0..5 {
        let mut changed = f.intent.clone();
        match variant {
            0 => changed.conversation_id = Uuid::new_v4(),
            1 => changed.manuscript_id = Uuid::new_v4(),
            2 => changed.input_id = Uuid::new_v4(),
            3 => changed.preview_request_id = Uuid::new_v4(),
            _ => changed.expected_queue_revision += 1,
        };
        assert!(matches!(
            f.db.queue_preview_cancel_status(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            f.db.cancel_queue_preview(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert_eq!(f.queue(), before);
    }
}

#[test]
fn closed_owner_keeps_committed_status_but_never_confirms_missing_proof_or_reopens_writes() {
    let f = Fixture::new();
    let (queue, _) = f.db.cancel_queue_preview(&f.intent).unwrap();
    f.db.shutdown().unwrap();
    assert!(
        matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Committed{queue:old,..} if old==queue)
    );
    assert!(f.db.cancel_queue_preview(&f.intent).is_err());
    let mut missing = f.intent.clone();
    missing.request_id = Uuid::new_v4();
    assert!(
        matches!(f.db.queue_preview_cancel_status(&missing).unwrap(),QueuePreviewCancelStatus::Unconfirmed(detail) if detail.safe_cause=="owner_closed_receipt_unconfirmed")
    );
}

#[test]
fn invalid_identity_and_nil_scope_are_rejected_before_any_sql_mutation() {
    let f = Fixture::new();
    let before = f.queue();
    for variant in 0..6 {
        let mut bad = f.intent.clone();
        match variant {
            0 => bad.request_id = Uuid::nil(),
            1 => bad.conversation_id = Uuid::nil(),
            2 => bad.manuscript_id = Uuid::nil(),
            3 => bad.input_id = Uuid::nil(),
            4 => bad.preview_request_id = Uuid::nil(),
            _ => bad.expected_queue_revision = 9_007_199_254_740_991,
        };
        assert!(matches!(
            f.db.cancel_queue_preview(&bad),
            Err(SqlImmediateFailure::Invalid(_))
        ));
        assert!(matches!(
            f.db.queue_preview_cancel_status(&bad),
            Err(SqlImmediateFailure::Invalid(_))
        ));
    }
    assert_eq!(f.queue(), before);
    assert_eq!(f.count(&f.observer()), (0, 0));
}

#[test]
fn status_does_not_materialize_an_accepted_journal_suffix() {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TaskKind},
    };
    use research_data_runtime::WriteOptions;
    let f = Fixture::new();
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.intent.conversation_id),
        1,
        0,
        f.record.request_id,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: f.record.turn_id,
            context_snapshot_id: f.record.context_snapshot_id,
            model: f.record.input.model.clone(),
        },
    )
    .unwrap();
    f.db.write(
        JournalCommand::new(
            ConversationId::from_uuid(f.intent.conversation_id),
            Uuid::new_v4(),
            0,
            vec![event],
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
    let observer = f.observer();
    let head = || {
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
    };
    assert_eq!(head(), 0);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    assert_eq!(head(), 0);
    assert_eq!(
        observer
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [f.intent.conversation_id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(f.count(&observer), (0, 0));
}
