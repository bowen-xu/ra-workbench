//! Real SQL proof/retention tests; all files are disposable and no provider runs.
use super::{journal::JournalCommand, *};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind, ModelIdentity, TaskKind},
};
use research_data_runtime::{
    Coordinator, CoordinatorConfig, LogConfig, Projector, RaftLog, ReadFence, WriteOptions,
};
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

fn config(path: &Path) -> LogConfig {
    let mut config = LogConfig::desktop(path);
    config.target_file_bytes = 64 * 1024;
    config.purge_threshold_bytes = 64 * 1024;
    config.rewrite_threshold_bytes = 64 * 1024;
    config.max_managed_file_bytes = 2 * 1024 * 1024;
    config.maintenance_reserve_bytes = 256 * 1024;
    config
}
fn sql(path: &Path, epoch: Uuid) -> Arc<Mutex<Connection>> {
    let mut sql = Connection::open(path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma journal_mode=wal;pragma synchronous=full;")
        .unwrap();
    sql.execute(
        "insert into data_runtime_state values(1,1,?1,0,'ready')",
        [epoch.to_string()],
    )
    .unwrap();
    Arc::new(Mutex::new(sql))
}
fn event(id: ConversationId, seq: u64, payload: EventPayload) -> AgentEvent {
    AgentEvent::new(id, seq, seq, Uuid::from_u128(42), payload).unwrap()
}
fn command(
    id: ConversationId,
    expected: u64,
    events: Vec<AgentEvent>,
    title: Option<String>,
) -> JournalCommand {
    JournalCommand::new(
        id,
        Uuid::new_v4(),
        expected,
        events,
        title,
        1_790_000_000_000,
    )
    .unwrap()
}
fn incompressible_text() -> String {
    let mut state = 0x4d595df4d0f33173_u64;
    (0..256 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"[(state % 62) as usize]
                as char
        })
        .collect()
}
fn files(path: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn query_reclaims_real_sql_prefix_after_a_busy_checkpoint_and_keeps_history_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data-log");
    let epoch = Uuid::new_v4();
    let raw = sql(&dir.path().join("fixture.sqlite3"), epoch);
    let owner = Coordinator::open(
        RaftLog::open_config(config(&path), Some(epoch)).unwrap(),
        journal::SqlProjector::new(raw.clone()),
        CoordinatorConfig {
            materialize_records: 4096,
            materialize_bytes: 2 * 1024 * 1024,
            reclaim_after_records: 2,
            ..CoordinatorConfig::default()
        },
    )
    .unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    owner
        .write(
            command(
                id,
                0,
                vec![
                    event(
                        id,
                        1,
                        EventPayload::TurnStarted {
                            task_kind: TaskKind::Regular,
                            turn_id: turn,
                            context_snapshot_id: Uuid::new_v4(),
                            model: ModelIdentity {
                                provider_id: "fixture".into(),
                                model_id: "fixture".into(),
                                revision: None,
                            },
                        },
                    ),
                    event(
                        id,
                        2,
                        EventPayload::ItemStarted {
                            turn_id: turn,
                            item_id: item,
                            kind: ItemKind::AssistantMessage,
                        },
                    ),
                ],
                Some("quota fixture".into()),
            ),
            WriteOptions::default(),
        )
        .unwrap();
    owner.query(|_, _| Ok(())).unwrap();
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    observer.execute_batch("begin;").unwrap();
    assert_eq!(
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    let body = incompressible_text();
    let deltas: Vec<_> = body
        .as_bytes()
        .chunks(65_536)
        .enumerate()
        .map(|(index, chunk)| {
            event(
                id,
                3 + index as u64,
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: std::str::from_utf8(chunk).unwrap().to_owned(),
                },
            )
        })
        .collect();
    let after_body = 2 + deltas.len() as u64;
    owner
        .write(command(id, 2, deltas, None), WriteOptions::default())
        .unwrap();
    for seq in after_body + 1..=after_body + 3 {
        owner
            .write(
                command(
                    id,
                    seq - 1,
                    vec![event(
                        id,
                        seq,
                        EventPayload::TextDelta {
                            turn_id: turn,
                            item_id: item,
                            text: "tail".into(),
                        },
                    )],
                    None,
                ),
                WriteOptions::default(),
            )
            .unwrap();
    }
    observer.execute_batch("commit;").unwrap();
    owner
        .query(|_, fence| {
            assert_eq!(fence.position, 5);
            Ok(())
        })
        .unwrap();
    let before = files(&path);
    for _ in 0..8 {
        owner.query(|_, _| Ok(())).unwrap();
    }
    assert_eq!(
        files(&path),
        before,
        "unchanged query does not append another sync/compact record"
    );
    drop(owner);
    let log = RaftLog::open_config(config(&path), Some(epoch)).unwrap();
    assert_eq!(log.head().unwrap().compacted_position, 5);
    let owner = Coordinator::open(
        log,
        journal::SqlProjector::new(raw.clone()),
        CoordinatorConfig::default(),
    )
    .unwrap();
    owner
        .query(|_, _| {
            let sql = raw.lock().unwrap();
            let saved: String = sql
                .query_row(
                    "select body from agent_messages where item_id=?1",
                    [item.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(saved, body + "tailtailtail");
            assert_eq!(
                sql.query_row("select count(*) from data_operation_receipts", [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                5
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn sql_reclaim_proof_requires_matching_full_durable_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let raw = sql(&dir.path().join("fixture.sqlite3"), epoch);
    let projector = journal::SqlProjector::new(raw.clone());
    let fence = ReadFence { epoch, position: 0 };
    assert!(projector.reclaim_fence(fence).unwrap().is_some());
    raw.lock()
        .unwrap()
        .execute_batch("pragma synchronous=normal;")
        .unwrap();
    assert!(projector.reclaim_fence(fence).is_err());
    raw.lock()
        .unwrap()
        .execute_batch("pragma synchronous=full;")
        .unwrap();
    assert!(
        projector
            .reclaim_fence(ReadFence { epoch, position: 1 })
            .is_err()
    );
    assert!(
        projector
            .reclaim_fence(ReadFence {
                epoch: Uuid::new_v4(),
                position: 0
            })
            .is_err()
    );
}
