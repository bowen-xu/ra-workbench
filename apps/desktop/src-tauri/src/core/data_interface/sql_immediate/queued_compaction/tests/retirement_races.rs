//! Both original writers keep their admission fence through the SQL commit.
use super::retirement::cancellation;
use super::*;
use crate::data_interface::QueuePreviewCancelStatus;
use rusqlite::functions::FunctionFlags;
use std::{sync::mpsc, thread, time::Duration};

enum Boundary {
    Start,
    Cancel,
}
struct SqlPause {
    entered: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
}
impl SqlPause {
    fn install(f: &Fixture, boundary: Boundary) -> Self {
        let (entered, observed) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let db = f.db.lock().unwrap();
        db.create_scalar_function(
            "pause_retirement_commit",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                let _ = entered.send(());
                released
                    .recv_timeout(Duration::from_secs(20))
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                Ok(0)
            },
        )
        .unwrap();
        let trigger = match boundary {
            Boundary::Start => {
                "create temp trigger pause_start_commit before insert on agent_journal_records when json_extract(new.payload_json,'$.payload.type')='turnStarted' begin select pause_retirement_commit(); end;"
            }
            Boundary::Cancel => {
                "create temp trigger pause_cancel_commit before insert on data_sql_operation_receipts when new.kind='agent.queue.preview.cancel' begin select pause_retirement_commit(); end;"
            }
        };
        db.execute_batch(trigger).unwrap();
        Self {
            entered: observed,
            release: Some(release),
        }
    }
    fn entered(&self) {
        self.entered.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    fn resume(mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
}
impl Drop for SqlPause {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}
fn start_command(f: &Fixture, state: &QueuedCompactionState) -> JournalCommand {
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.conversation),
        1,
        0,
        state.record.request_id,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Compact,
            turn_id: state.record.turn_id,
            context_snapshot_id: state.record.context_snapshot_id,
            model: state.record.input.model.clone(),
        },
    )
    .unwrap();
    JournalCommand::new(
        ConversationId::from_uuid(f.conversation),
        Uuid::new_v4(),
        0,
        vec![event],
        None,
        1_790_000_000_000,
    )
    .unwrap()
}
fn item_count(f: &Fixture) -> u64 {
    let observer = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
    observer
        .query_row(
            "select count(*) from agent_items where conversation_id=?1",
            [f.conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn start_commit_holds_the_original_fence_until_cancel_can_observe_the_admitted_turn() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    let command = start_command(&f, &state);
    let pause = SqlPause::install(&f, Boundary::Start);
    let writer = f.db.clone();
    let starting = thread::spawn(move || {
        writer.write(
            command,
            WriteOptions {
                mode: WriteMode::Immediate,
                ..Default::default()
            },
        )
    });
    pause.entered();
    let (entered, observed) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    let writer = f.db.clone();
    let original = intent.clone();
    let canceling = thread::spawn(move || {
        entered.send(()).unwrap();
        completed
            .send(writer.cancel_queue_preview(&original))
            .unwrap();
    });
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    pause.resume();
    assert!(starting.join().unwrap().unwrap().materialized);
    assert!(matches!(
        result.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    canceling.join().unwrap();
    assert_eq!(f.q(), before);
    assert_eq!(item_count(&f), 1);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    assert!(
        matches!(f.read(Some(prepare.request_id)), Some(QueuedCompactionRead::Compact(state)) if state.current && state.retirement.is_none())
    );
}

#[test]
fn cancellation_commit_holds_the_original_fence_and_rejects_the_late_start_before_log_acceptance() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    let accepted = f.db.benchmark_snapshot().unwrap().accepted_position;
    let command = start_command(&f, &state);
    let pause = SqlPause::install(&f, Boundary::Cancel);
    let writer = f.db.clone();
    let original = intent.clone();
    let canceling = thread::spawn(move || writer.cancel_queue_preview(&original));
    pause.entered();
    let (entered, observed) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    let writer = f.db.clone();
    let starting = thread::spawn(move || {
        entered.send(()).unwrap();
        completed
            .send(writer.write(
                command,
                WriteOptions {
                    mode: WriteMode::Immediate,
                    ..Default::default()
                },
            ))
            .unwrap();
    });
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    pause.resume();
    let (released, proof) = canceling.join().unwrap().unwrap();
    assert_eq!(proof.operation_id, intent.request_id);
    assert!(
        result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_err()
    );
    starting.join().unwrap();
    assert_eq!(
        f.db.benchmark_snapshot().unwrap().accepted_position,
        accepted
    );
    assert_eq!(item_count(&f), 0);
    assert_eq!(released.inputs(), before.inputs());
    assert!(released.paused());
    assert_eq!(released.preparing(), None);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::Committed { .. }
    ));
    assert!(
        matches!(f.read(Some(prepare.request_id)), Some(QueuedCompactionRead::Compact(state)) if !state.current && state.retirement.is_some())
    );
}
