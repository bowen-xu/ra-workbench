//! Old R=I standalone preparations never imply a new queue-attempt claim.
use super::*;
use crate::{
    agent::writing_context::{WritingContextFile, WritingContextRequest},
    agent::{
        repository::input_queue,
        writing_submission::{
            FrozenWritingModel, WritingSubmissionRequest, materialize_submission_in,
        },
    },
    data_interface::{Database, SharedDatabase, SqlImmediateFailure, journal::JournalCommand},
    storage::AppStore,
};
use research_agent_core::{conversation::ConversationId, protocol::ModelIdentity};
use research_data_runtime::{WriteMode, WriteOptions};
use sha2::{Digest, Sha256};

struct Fixture {
    _root: tempfile::TempDir,
    db: SharedDatabase,
    store: AppStore,
    conversation: Uuid,
    manuscript: Uuid,
    input: Uuid,
    original: SubmissionRecord,
}
impl Fixture {
    fn new(compact_header: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("legacy.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic legacy compact')",
            [conversation.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('legacy-project','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'legacy-project','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![conversation.to_string(),manuscript.to_string()]).unwrap();
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',17,?2,'now')",params![manuscript.to_string(),format!("{:x}",Sha256::digest(b"Frozen manuscript"))]).unwrap();
        input_queue::apply_checked(
            &mut sql,
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
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        let source = store
            .manuscript_directory()
            .join(manuscript.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
        let original = {
            let mut sql = db.lock().unwrap();
            let tx = sql.transaction().unwrap();
            let mut request = Self::request(conversation, manuscript, input);
            if !compact_header {
                request.text = "Old ordinary submission".into();
            }
            let materialized =
                materialize_submission_in(&tx, &store, request, Self::model()).unwrap();
            let record = submissions::insert_in(&tx, conversation, input, materialized).unwrap();
            if compact_header {
                crate::agent::writing_submission::freeze_compaction_in(&tx, &record, Some(0))
                    .unwrap();
            }
            tx.commit().unwrap();
            record
        };
        Self {
            _root: root,
            db,
            store,
            conversation,
            manuscript,
            input,
            original,
        }
    }
    fn request(conversation: Uuid, manuscript: Uuid, request: Uuid) -> WritingSubmissionRequest {
        WritingSubmissionRequest {
            conversation_id: conversation,
            request_id: request,
            text: "/compact".into(),
            user_input: vec![],
            context: WritingContextRequest {
                manuscript_id: manuscript,
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
        }
    }
    fn model() -> FrozenWritingModel {
        FrozenWritingModel {
            identity: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "synthetic".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
        }
    }
    fn queue(&self) -> InputQueue {
        input_queue::read_checked(&mut self.db.lock().unwrap(), self.conversation, |_| Ok(()))
            .unwrap()
    }
    fn intent(&self) -> PrepareIntent {
        let q = self.queue();
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
        let request = Uuid::new_v4();
        PrepareIntent {
            conversation_id: self.conversation,
            manuscript_id: self.manuscript,
            input_id: self.input,
            request_id: request,
            expected_queue_revision: q.revision(),
            expected_revision: w,
            previous_request_id: None,
            request_json: serde_json::json!({"inputId":self.input,"expectedQueueRevision":q.revision(),"previousRequestId":null,"submission":{"conversationId":self.conversation,"requestId":request,"text":"/compact","context":{"manuscriptId":self.manuscript,"files":[]},"model":null},"expectedRevision":w}),
        }
    }
    fn read(&self) -> Result<Option<QueuedCompactionRead>, SqlImmediateFailure> {
        self.db.read_queued_compaction(&ReadIntent {
            conversation_id: self.conversation,
            manuscript_id: self.manuscript,
            input_id: self.input,
            request_id: None,
        })
    }
    fn prepare(
        &self,
        intent: &PrepareIntent,
    ) -> Result<QueuedCompactionState, SqlImmediateFailure> {
        self.db.prepare_queued_compaction(
            intent,
            PrepareSource {
                store: self.store.clone(),
                request: Self::request(self.conversation, self.manuscript, intent.request_id),
                model: Self::model(),
            },
        )
    }
    fn terminal(&self, outcome: TurnOutcome) {
        let payloads = vec![
            EventPayload::TurnStarted {
                task_kind: TaskKind::Compact,
                turn_id: self.original.turn_id,
                context_snapshot_id: self.original.context_snapshot_id,
                model: self.original.input.model.clone(),
            },
            EventPayload::TurnFinished {
                turn_id: self.original.turn_id,
                outcome,
            },
        ];
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(n, payload)| {
                AgentEvent::new(
                    ConversationId::from_uuid(self.conversation),
                    n as u64 + 1,
                    0,
                    self.input,
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
                    0,
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
    fn assert_no_claim_or_new_request(&self, new_request: Uuid) {
        let db = self.db.lock().unwrap();
        for query in [
            "select count(*) from agent_submissions where request_id=?1",
            "select count(*) from agent_input_queue_receipts where request_id=?1",
            "select count(*) from data_sql_operation_receipts where operation_id=?1",
        ] {
            let count: u64 = db
                .query_row(query, [new_request.to_string()], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        for table in [
            "agent_queue_submission_origins",
            "agent_input_queue_previews",
            "agent_input_queue_admissions",
        ] {
            let count: u64 = db
                .query_row(&format!("select count(*) from {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
    }
}

#[test]
fn legacy_unstarted_failed_and_completed_compacts_cannot_be_silently_claimed_again() {
    for outcome in [
        None,
        Some(TurnOutcome::Failed),
        Some(TurnOutcome::Completed),
    ] {
        let f = Fixture::new(true);
        if let Some(outcome) = outcome {
            f.terminal(outcome);
        }
        let before = f.queue();
        let intent = f.intent();
        assert!(matches!(
            f.read(),
            Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
        ));
        assert!(matches!(
            f.prepare(&intent),
            Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
        ));
        assert_eq!(f.queue(), before);
        assert_eq!(before.inputs()[0].id, f.input);
        assert_eq!(before.preparing(), None);
        f.assert_no_claim_or_new_request(intent.request_id);
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
            input_queue::list_unstarted_checked(
                &mut f.db.lock().unwrap(),
                f.conversation,
                None,
                64,
                |_| Ok(())
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn ordinary_r_equal_i_without_compact_header_does_not_block_a_fresh_queue_attempt() {
    let f = Fixture::new(false);
    assert!(f.read().unwrap().is_none());
    let intent = f.intent();
    let state = f.prepare(&intent).unwrap();
    assert_eq!(state.record.request_id, intent.request_id);
    assert_ne!(state.record.request_id, f.input);
    assert_eq!(state.queue.preparing(), Some(f.input));
    assert_eq!(state.queue.inputs()[0].id, f.input);
}
