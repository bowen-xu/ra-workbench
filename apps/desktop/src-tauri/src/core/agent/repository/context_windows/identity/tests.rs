use super::*;
use crate::agent::repository::context_windows::tests::{append, fixture};
use research_agent_core::protocol::{EventPayload, TurnOutcome};

fn setup() -> (Connection, ContextCheckpoint) {
    let mut db = Connection::open_in_memory().unwrap();
    let checkpoint = fixture(&mut db);
    db.execute_batch(include_str!(
        "../../../../../../migrations/0135_agent_initial_context_windows.sql"
    ))
    .unwrap();
    (db, checkpoint)
}

#[test]
fn identity_tracks_only_active_chain_and_keeps_initial_uuid_after_rollback_to_empty() {
    let (mut db, mut checkpoint) = setup();
    let initial = Uuid::now_v7();
    let first = read_or_create(&mut db, checkpoint.conversation_id, initial).unwrap();
    assert_eq!(
        first,
        WindowIdentity {
            number: 0,
            first: initial,
            previous: None,
            current: initial
        }
    );
    install(&mut db, &checkpoint).unwrap();
    let original_checkpoint = checkpoint.clone();
    let one = read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap();
    assert_eq!(one.number, 1);
    assert_eq!(one.previous, Some(initial));
    assert_eq!(one.current, checkpoint.window_id);
    checkpoint.previous_window_id = Some(checkpoint.window_id);
    checkpoint.window_id = Uuid::now_v7();
    install(&mut db, &checkpoint).unwrap();
    let two = read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap();
    assert_eq!(two.number, 2);
    assert_eq!(two.first, initial);
    assert_eq!(two.previous, Some(one.current));
    assert_eq!(
        install(&mut db, &original_checkpoint).unwrap(),
        Installation::AlreadyInstalled { active: false }
    );
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap(),
        two
    );
    // This is a pointer selection, not deletion: all immutable records remain.
    db.execute(
        "update agent_active_context_windows set window_id=?2 where conversation_id=?1",
        params![
            checkpoint.conversation_id.to_string(),
            one.current.to_string()
        ],
    )
    .unwrap();
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap(),
        one
    );
    db.execute(
        "delete from agent_active_context_windows where conversation_id=?1",
        [checkpoint.conversation_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap(),
        first
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    append(
        &mut db,
        checkpoint.conversation_id,
        1,
        EventPayload::TurnFinished {
            turn_id: checkpoint.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    let raw = db
        .query_row(
            "select window_id from agent_initial_context_windows",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(raw, initial.to_string());
    assert!(
        db.execute(
            "update agent_initial_context_windows set window_id=?1",
            [Uuid::now_v7().to_string()]
        )
        .is_err()
    );
}

#[test]
fn corrupt_chain_rolls_back_new_initial_identity_and_invalid_ids_never_write() {
    let (mut db, mut checkpoint) = setup();
    checkpoint.previous_window_id = Some(checkpoint.window_id);
    let encoded = serde_json::to_string(&checkpoint).unwrap();
    db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) values(?1,?2,?3,1,?1,1,?4)", params![checkpoint.window_id.to_string(), checkpoint.conversation_id.to_string(), checkpoint.turn_id.to_string(), encoded]).unwrap();
    db.execute(
        "insert into agent_active_context_windows(conversation_id,window_id) values(?1,?2)",
        params![
            checkpoint.conversation_id.to_string(),
            checkpoint.window_id.to_string()
        ],
    )
    .unwrap();
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()),
        Err(Error::Invalid)
    );
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::nil()),
        Err(Error::Invalid)
    );
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::new_v4()),
        Err(Error::Invalid)
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_initial_context_windows",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn production_upgrade_and_file_reopen_preserve_initial_id_and_checkpoint_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("window-identity.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let checkpoint = fixture(&mut db);
    install(&mut db, &checkpoint).unwrap();
    let original: String = db
        .query_row(
            "select checkpoint_json from agent_context_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let identity = read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap();
    assert_eq!(identity.number, 1);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    assert_eq!(
        read_or_create(&mut db, checkpoint.conversation_id, Uuid::now_v7()).unwrap(),
        identity
    );
    assert_eq!(
        db.query_row(
            "select checkpoint_json from agent_context_checkpoints",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        original
    );
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=135",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
