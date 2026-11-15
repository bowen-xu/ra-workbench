use super::*;
#[path = "tests/context.rs"]
mod context;
use crate::{
    agent::{
        repository::input_queue,
        writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
    },
    data_interface::SharedDatabase,
    storage::AppStore,
};
use research_agent_core::{
    conversation::CatalogError,
    input_queue::{InputQueue, QueueEdit, QueuedInput},
    protocol::ModelIdentity,
    user_input::UserInput,
};
use rusqlite::{Connection, params};
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "tests/compatibility.rs"]
mod compatibility;
#[path = "tests/failures.rs"]
mod failures;
#[path = "tests/frozen_scope.rs"]
mod frozen_scope;
#[path = "tests/inspection.rs"]
mod inspection;
#[path = "tests/materialization.rs"]
mod materialization;
#[path = "../queued_start_status/tests.rs"]
mod start_status;

struct Fixture {
    root: tempfile::TempDir,
    db: SharedDatabase,
    store: AppStore,
    intent: QueuePreviewPrepareIntent,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        let request = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic ordinary prepare')",
            [conversation.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![conversation.to_string(),manuscript.to_string()]).unwrap();
        let hash = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',17,?2,'now')",params![manuscript.to_string(),hash]).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        let source = store
            .manuscript_directory()
            .join(manuscript.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
        let text = "Explain synthetic references🙂";
        let user_input = vec![UserInput::Text {
            text: text.into(),
            text_elements: vec![],
        }];
        let f = Self {
            root,
            db,
            store,
            intent: QueuePreviewPrepareIntent {
                conversation_id: conversation,
                manuscript_id: manuscript,
                input_id: input,
                request_id: request,
                expected_queue_revision: 1,
                request_json: json!({"inputId":input,"expectedRevision":1,"submission":{
                    "conversationId":conversation,"requestId":request,"text":text,"userInput":user_input,
                    "context":{"manuscriptId":manuscript,"files":[{"relativePath":"main.tex","expectedDiskSha256":hash,"buffer":null,"selection":null}]},
                    "resources":[],"retrieval":null,"model":null,"sessionConfig":null,"turnConfig":null
                }}),
            },
        };
        f.edit(QueueEdit::Add(QueuedInput {
            id: input,
            text: text.into(),
            user_input,
            origin: None,
        }));
        f
    }
    fn source(&self) -> QueuePreviewPrepareSource {
        self.source_for(&self.intent)
    }
    fn source_for(&self, intent: &QueuePreviewPrepareIntent) -> QueuePreviewPrepareSource {
        let raw = &intent.request_json["submission"];
        QueuePreviewPrepareSource {
            store: self.store.clone(),
            request: WritingSubmissionRequest {
                conversation_id: intent.conversation_id,
                request_id: intent.request_id,
                text: raw["text"].as_str().unwrap().into(),
                user_input: serde_json::from_value(raw["userInput"].clone()).unwrap(),
                context: serde_json::from_value(raw["context"].clone()).unwrap(),
                resources: serde_json::from_value(raw["resources"].clone()).unwrap(),
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
    fn prepare(&self) -> QueuePreviewPrepared {
        self.db
            .prepare_queue_preview(&self.intent, self.source())
            .unwrap()
    }
    fn queue(&self) -> InputQueue {
        input_queue::read_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            |_| Ok(()),
        )
        .unwrap()
    }
    fn edit(&self, edit: QueueEdit) {
        let revision = self.queue().revision();
        input_queue::apply_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            Uuid::new_v4(),
            revision,
            edit,
            |_| Ok(()),
        )
        .unwrap();
    }
    fn replace_input(&mut self, user_input: Vec<UserInput>) {
        let text = self.intent.request_json["submission"]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        self.edit(QueueEdit::Update {
            id: self.intent.input_id,
            text,
            user_input: user_input.clone(),
        });
        self.intent.expected_queue_revision = self.queue().revision();
        self.intent.request_json["expectedRevision"] = json!(self.intent.expected_queue_revision);
        self.intent.request_json["submission"]["userInput"] = json!(user_input);
    }
    fn observer(&self) -> Connection {
        Connection::open(self.root.path().join("prepare.sqlite3")).unwrap()
    }
    fn counts(&self, db: &Connection) -> (u64, u64, u64) {
        let count = |table: &str, field: &str| {
            db.query_row(
                &format!("select count(*) from {table} where {field}=?1"),
                [self.intent.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap()
        };
        (
            count("agent_submissions", "request_id"),
            count("agent_input_queue_receipts", "request_id"),
            count("data_sql_operation_receipts", "operation_id"),
        )
    }
    fn reopen(self) -> Self {
        let Self {
            root,
            db,
            store,
            intent,
        } = self;
        drop(store);
        drop(db);
        let mut sql = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        Self {
            root,
            db,
            store,
            intent,
        }
    }
}

#[test]
fn typed_prepare_freezes_q_plus_one_and_replay_does_not_reinstall_a_canceled_old_claim() {
    let f = Fixture::new();
    assert!(matches!(
        f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),
        QueuePreviewPrepareStatus::NotFound
    ));
    assert!(
        f.db.replay_queue_preview_prepare(&f.intent)
            .unwrap()
            .is_none()
    );
    let original = f.prepare();
    assert_eq!(original.queue.revision(), 2);
    assert_eq!(original.queue.preparing(), Some(f.intent.input_id));
    assert_eq!(
        original.record.input.user_input,
        f.queue().inputs()[0].user_input
    );
    input_queue::cancel_preview_checked(
        &mut f.db.lock().unwrap(),
        f.intent.conversation_id,
        Uuid::new_v4(),
        2,
        f.intent.input_id,
        f.intent.request_id,
        |_| Ok(()),
    )
    .unwrap();
    f.edit(QueueEdit::Resume);
    let current_q = f.queue().revision();
    let newer = Uuid::new_v4();
    input_queue::prepare_preview_checked(
        &mut f.db.lock().unwrap(),
        f.intent.conversation_id,
        newer,
        current_q,
        f.intent.input_id,
        original.record.input.clone(),
        |_| Ok(()),
    )
    .unwrap();
    let current = f.queue();
    let replay =
        f.db.replay_queue_preview_prepare(&f.intent)
            .unwrap()
            .unwrap();
    assert_eq!(replay.record, original.record);
    assert_eq!(replay.queue, original.queue);
    assert_eq!(f.prepare().queue, original.queue);
    assert_eq!(f.queue(), current);
    let f = f.reopen();
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(), QueuePreviewPrepareStatus::Committed {state,receipt} if state.queue==original.queue&&state.record==original.record&&receipt.replayed)
    );
    assert_eq!(f.queue(), current);
    assert_eq!(f.counts(&f.observer()), (1, 1, 1));
}

#[test]
fn admitted_preview_replays_original_queue_after_input_was_consumed() {
    let f = Fixture::new();
    let original = f.prepare();
    input_queue::admit_checked(
        &mut f.db.lock().unwrap(),
        f.intent.conversation_id,
        f.intent.request_id,
        original.queue.revision(),
        f.intent.input_id,
        original.record.input.clone(),
        |_| Ok(()),
    )
    .unwrap();
    assert!(f.queue().inputs().is_empty());
    assert_eq!(f.prepare().queue, original.queue);
    assert!(f.queue().inputs().is_empty());
}
