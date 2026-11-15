use super::*;
use crate::data_interface::SharedDatabase;
use research_agent_core::{
    input_queue::{QueueEdit, QueueError, QueuedInput},
    user_input::UserInput,
};
use rusqlite::params;

#[path = "tests/compatibility.rs"]
mod compatibility;
#[path = "tests/failures.rs"]
mod failures;
#[path = "tests/ordering.rs"]
mod ordering;
#[path = "tests/upgrade.rs"]
mod upgrade;

struct Fixture {
    root: tempfile::TempDir,
    db: SharedDatabase,
    intent: QueueEditIntent,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let conversation = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic queue edit')",
            [conversation.to_string()],
        )
        .unwrap();
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        Self {
            root,
            db,
            intent: QueueEditIntent {
                conversation_id: conversation,
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                expected_epoch: None,
                scope: QueueEditScope::Context,
                edit: QueueEdit::Add(QueuedInput {
                    id: Uuid::new_v4(),
                    text: "synthetic queue🙂".into(),
                    user_input: vec![UserInput::Text {
                        text: "synthetic queue🙂".into(),
                        text_elements: vec![],
                    }],
                    origin: None,
                }),
            },
        }
    }
    fn next(&self, revision: u64, edit: QueueEdit) -> QueueEditIntent {
        QueueEditIntent {
            request_id: Uuid::new_v4(),
            expected_revision: revision,
            edit,
            ..self.intent.clone()
        }
    }
    fn observer(&self) -> Connection {
        Connection::open(self.root.path().join("queue.sqlite3")).unwrap()
    }
    fn queue(&self) -> InputQueue {
        input_queue::read_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            |_| Ok(()),
        )
        .unwrap()
    }
    fn count(&self) -> (u64, u64) {
        let sql = self.observer();
        (
            sql.query_row(
                "select count(*) from agent_input_queue_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            sql.query_row(
                "select count(*) from data_sql_operation_receipts where kind=?1",
                [QUEUE_EDIT_FAMILY],
                |row| row.get(0),
            )
            .unwrap(),
        )
    }
    fn proof(&self) -> (Vec<u8>, String, u64, String) {
        self.observer().query_row("select fingerprint,epoch,sql_fence_position,result_json from data_sql_operation_receipts where operation_id=?1",
            [self.intent.request_id.to_string()], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
    }
    fn legacy(&self) -> InputQueue {
        input_queue::apply_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            self.intent.request_id,
            self.intent.expected_revision,
            self.intent.lowered(),
            |_| Ok(()),
        )
        .unwrap()
    }
    fn reopen(self) -> Self {
        let Self { root, db, intent } = self;
        drop(db);
        let sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        Self { root, db, intent }
    }
}
fn committed(outcome: QueueEditOutcome) -> (InputQueue, SqlImmediateReceipt) {
    match outcome {
        QueueEditOutcome::Committed { queue, receipt } => (queue, receipt),
        QueueEditOutcome::LegacyReplay(_) => panic!("typed proof required"),
    }
}

#[test]
fn expected_epoch_guards_mutation_and_receipt_identity_inside_owner() {
    let f = Fixture::new();
    let (_, first) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    let mut remove = f.next(
        1,
        QueueEdit::Remove(match &f.intent.edit {
            QueueEdit::Add(input) => input.id,
            _ => unreachable!(),
        }),
    );
    remove.expected_epoch = Some(Uuid::new_v4());
    let before = f.queue();
    assert!(matches!(
        f.db.edit_input_queue(&remove),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
    ));
    assert!(matches!(
        f.db.input_queue_edit_status(&remove),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
    ));
    assert_eq!(f.queue(), before);
    assert_eq!(f.count(), (1, 1));
    remove.expected_epoch = Some(first.epoch);
    let (queue, receipt) = committed(f.db.edit_input_queue(&remove).unwrap());
    assert!(queue.inputs().is_empty());
    assert_eq!(receipt.epoch, first.epoch);
    assert!(matches!(
        f.db.input_queue_edit_status(&remove),
        Ok(QueueEditStatus::Committed { .. })
    ));
    let (_, replay) = committed(f.db.edit_input_queue(&remove).unwrap());
    assert!(replay.replayed);
    let mut different_identity = remove.clone();
    different_identity.expected_epoch = None;
    assert!(matches!(
        f.db.edit_input_queue(&different_identity),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
    ));
    assert!(matches!(
        f.db.input_queue_edit_status(&different_identity),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
    ));
    assert_eq!(f.count(), (2, 2));
}

#[test]
fn original_queue_edit_replay_and_cold_status_preserve_w0_proof_and_never_restore_current_queue() {
    let f = Fixture::new();
    let (original, receipt) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    assert_eq!(receipt.sql_fence.position, 0);
    assert!(!receipt.replayed);
    assert_eq!(f.proof().3, "null");
    let pause = f.next(1, QueueEdit::Pause);
    let (current, second) = committed(f.db.edit_input_queue(&pause).unwrap());
    assert_eq!(second.sql_fence, receipt.sql_fence);
    assert_eq!(f.count(), (2, 2));
    let (replayed, proof) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    assert!(proof.replayed);
    assert_eq!(replayed, original);
    assert_eq!(f.queue(), current);
    let bytes = f.proof();
    let f = f.reopen();
    let changes = f.db.raw.lock().unwrap().total_changes();
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(), QueueEditStatus::Committed {queue,receipt:proof} if queue==original && proof.sql_fence==receipt.sql_fence)
    );
    assert_eq!(f.db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(f.proof(), bytes);
    assert_eq!(f.queue(), current);
}

#[test]
fn exact_restore_replay_uses_its_frozen_result_after_original_source_receipts_are_gone() {
    let f = Fixture::new();
    let original = committed(f.db.edit_input_queue(&f.intent).unwrap()).0;
    let source = original.inputs()[0].clone();
    f.db.edit_input_queue(&f.next(1, QueueEdit::PauseInput(source.id)))
        .unwrap();
    f.db.edit_input_queue(&f.next(2, QueueEdit::Remove(source.id)))
        .unwrap();
    let restored = QueuedInput {
        id: Uuid::new_v4(),
        ..source.clone()
    };
    let intent = f.next(
        3,
        QueueEdit::Restore {
            input: restored.clone(),
            source_id: source.id,
            index: 0,
            paused: false,
        },
    );
    let (queue, _) = committed(f.db.edit_input_queue(&intent).unwrap());
    assert_eq!(queue.inputs()[0], restored);
    assert!(queue.input_paused(restored.id));
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from agent_input_queue_receipts where request_id<>?1",
            [intent.request_id.to_string()],
        )
        .unwrap();
    let before = f.db.raw.lock().unwrap().total_changes();
    assert_eq!(committed(f.db.edit_input_queue(&intent).unwrap()).0, queue);
    assert!(
        matches!(f.db.input_queue_edit_status(&intent).unwrap(),QueueEditStatus::Committed{queue:saved,..} if saved==queue)
    );
    assert_eq!(f.db.raw.lock().unwrap().total_changes(), before);
    let f = f.reopen();
    assert_eq!(committed(f.db.edit_input_queue(&intent).unwrap()).0, queue);
}

#[test]
fn original_identity_cannot_change_text_scope_revision_or_cross_sql_and_journal_families() {
    let f = Fixture::new();
    f.db.edit_input_queue(&f.intent).unwrap();
    let mut text = f.intent.clone();
    if let QueueEdit::Add(input) = &mut text.edit {
        input.text.push('!');
    }
    let mut revision = f.intent.clone();
    revision.expected_revision = 1;
    let mut scope = f.intent.clone();
    scope.scope = QueueEditScope::Paper {
        paper_id: "other".into(),
    };
    for changed in [text, revision, scope] {
        assert!(matches!(
            f.db.edit_input_queue(&changed),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
        ));
        assert!(matches!(
            f.db.input_queue_edit_status(&changed),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
        ));
    }
    assert_eq!(f.count(), (1, 1));
    for journal in [false, true] {
        let f = Fixture::new();
        if journal {
            f.db.raw.lock().unwrap().execute("insert into data_operation_receipts(operation_id,epoch,position,fingerprint,result_json) select ?1,epoch,1,?2,'0' from data_runtime_state",params![f.intent.request_id.to_string(),[9u8;32].as_slice()]).unwrap();
        } else {
            f.db.raw.lock().unwrap().execute("insert into data_sql_operation_receipts(operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json) select ?1,epoch,'other.family',1,?2,materialized_position,'null' from data_runtime_state",params![f.intent.request_id.to_string(),[8u8;32].as_slice()]).unwrap();
        }
        assert!(matches!(
            f.db.edit_input_queue(&f.intent),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
        ));
        assert!(matches!(
            f.db.input_queue_edit_status(&f.intent),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Conflict(_)))
        ));
        assert_eq!(f.count(), (0, 0));
    }
}
