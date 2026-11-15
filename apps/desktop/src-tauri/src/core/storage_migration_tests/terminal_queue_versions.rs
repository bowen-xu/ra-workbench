//! The deployed 213 trigger must retain every queue snapshot through upgrade.
use super::migrate_through;
use crate::agent::repository::{append_batch, input_queue};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TurnOutcome},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use uuid::Uuid;

fn latest(db: &Connection) -> i64 {
    db.query_row("select max(version) from schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn latest_registered() -> i64 {
    crate::storage_migrations::MIGRATIONS.last().unwrap().0
}
fn conversation(db: &Connection) -> Uuid {
    let c = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Synthetic terminal queue')",
        [c.to_string()],
    )
    .unwrap();
    c
}
fn snapshot(c: Uuid, version: u64, revision: i64, paused: bool) -> Value {
    let head = Uuid::new_v4();
    let tail = Uuid::new_v4();
    let mut value = json!({"version":version,"conversation":c,"revision":revision,"paused":paused,"inputs":[[head,"/compact"],[tail,"保留后续输入🙂"]],"preparing":head});
    if version >= 2 {
        value["origins"] = json!([[head,{"kind":"manuscript","resourceId":Uuid::new_v4()}],[tail,{"kind":"note","resourceId":Uuid::new_v4()}]]);
    }
    if version >= 3 {
        value["user_inputs"] = json!([[head,[{"type":"text","text":"/compact","textElements":[]},{"type":"annotation","source":"source://synthetic/evidence","title":"Frozen evidence","text":"preserved evidence","comment":"retained comment"}]]]);
    }
    if version >= 4 {
        value["paused_inputs"] = json!([tail]);
    }
    value
}
fn insert_queue(db: &Connection, c: Uuid, revision: i64, value: &Value) {
    db.execute(
        "insert into agent_input_queues(conversation_id,revision,snapshot_json) values(?1,?2,?3)",
        params![c.to_string(), revision, value.to_string()],
    )
    .unwrap();
}
fn row(db: &Connection, c: Uuid) -> (i64, String) {
    db.query_row(
        "select revision,snapshot_json from agent_input_queues where conversation_id=?1",
        [c.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}
fn terminal(c: Uuid, outcome: TurnOutcome) -> AgentEvent {
    AgentEvent::new(
        ConversationId::from_uuid(c),
        1,
        0,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: Uuid::new_v4(),
            outcome,
        },
    )
    .unwrap()
}
fn append(
    db: &mut Connection,
    c: Uuid,
    batch: Uuid,
    event: &AgentEvent,
) -> Result<u64, crate::agent::repository::JournalError> {
    append_batch(
        db,
        ConversationId::from_uuid(c),
        batch,
        0,
        std::slice::from_ref(event),
    )
}
fn checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=213 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn trigger(db: &Connection) -> String {
    db.query_row(
        "select sql from sqlite_schema where name='agent_terminal_queue_pause'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}
fn unchanged_history(db: &Connection, c: Uuid) {
    assert_eq!(
        db.query_row(
            "select durable_count from agent_conversations where id=?1",
            [c.to_string()],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    for query in [
        "select count(*) from agent_journal_records where conversation_id=?1",
        "select count(*) from agent_journal_batches where conversation_id=?1",
        "select count(*) from data_journal_operation_ids where conversation_id=?1",
    ] {
        assert_eq!(
            db.query_row(query, [c.to_string()], |row| row.get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert!(db.is_autocommit());
}

#[test]
fn real_213_upgrade_preserves_v1_to_v4_payloads_origins_holds_and_claims_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("terminal-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 213);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let old_checksums = checksums(&db);
    let mut fixtures = vec![];
    for version in 1..=4 {
        for paused in [false, true] {
            let c = conversation(&db);
            let value = snapshot(c, version, 7, paused);
            insert_queue(&db, c, 7, &value);
            // Prove the deployed fixture is valid before attributing any later
            // decoding failure to the migration's lossless JSON update.
            input_queue::read_checked(&mut db, c, |_| Ok(()))
                .unwrap_or_else(|error| panic!("invalid synthetic v{version} fixture: {error}"));
            fixtures.push((c, version, paused, value));
        }
    }
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(latest(&db), latest_registered());
    assert_eq!(checksums(&db), old_checksums);
    for (c, version, paused, value) in &fixtures {
        assert_eq!(row(&db, *c), (7, value.to_string()));
        let outcome = match version {
            1 => TurnOutcome::Failed,
            2 => TurnOutcome::Canceled,
            _ => TurnOutcome::Interrupted,
        };
        let event = terminal(*c, outcome);
        let batch = Uuid::new_v4();
        assert_eq!(append(&mut db, *c, batch, &event).unwrap(), 1);
        assert_eq!(append(&mut db, *c, batch, &event).unwrap(), 1);
        let mut expected = value.clone();
        if !paused {
            expected["revision"] = json!(8);
            expected["paused"] = json!(true);
        }
        let (revision, stored) = row(&db, *c);
        assert_eq!(revision, if *paused { 7 } else { 8 });
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), expected);
        if *paused {
            assert_eq!(stored, value.to_string());
        }
        let restored = input_queue::read_checked(&mut db, *c, |_| Ok(())).unwrap();
        assert!(restored.paused());
        assert_eq!(
            restored.preparing().unwrap().to_string(),
            value["preparing"].as_str().unwrap()
        );
        assert_eq!(restored.inputs()[1].text, "保留后续输入🙂");
        if *version >= 3 {
            assert_eq!(
                serde_json::to_value(&restored.inputs()[0].user_input).unwrap(),
                value["user_inputs"][0][1]
            );
        }
        if *version == 4 {
            assert!(restored.input_paused(restored.inputs()[1].id));
        }
    }
    let frozen: Vec<_> = fixtures.iter().map(|(c, ..)| (*c, row(&db, *c))).collect();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), old_checksums);
    for (c, expected) in frozen {
        assert_eq!(row(&db, c), expected);
    }
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn original_213_rejects_v3_v4_terminals_then_215_commits_the_same_original_batch() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 213);
    for version in [3, 4] {
        let c = conversation(&db);
        let value = snapshot(c, version, 7, false);
        insert_queue(&db, c, 7, &value);
        let event = terminal(c, TurnOutcome::Failed);
        let batch = Uuid::new_v4();
        assert!(append(&mut db, c, batch, &event).is_err());
        assert_eq!(row(&db, c), (7, value.to_string()));
        unchanged_history(&db, c);
    }
    // Use a separate deployed pair per original batch so its identity remains fixed.
    for version in [3, 4] {
        let mut db = Connection::open_in_memory().unwrap();
        migrate_through(&mut db, 213);
        let c = conversation(&db);
        let value = snapshot(c, version, 7, false);
        insert_queue(&db, c, 7, &value);
        let event = terminal(c, TurnOutcome::Failed);
        let batch = Uuid::new_v4();
        assert!(append(&mut db, c, batch, &event).is_err());
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(append(&mut db, c, batch, &event).unwrap(), 1);
        let stored: Value = serde_json::from_str(&row(&db, c).1).unwrap();
        assert_eq!(stored["revision"], 8);
        assert_eq!(stored["paused"], true);
        for field in [
            "inputs",
            "origins",
            "user_inputs",
            "paused_inputs",
            "preparing",
        ] {
            assert_eq!(stored[field], value[field]);
        }
    }
}

#[test]
fn unsupported_or_bad_headers_and_unpaused_revision_overflow_abort_the_entire_terminal() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    for case in 0..9 {
        let c = conversation(&db);
        let revision = if case == 8 { i64::MAX } else { 7 };
        let mut value = snapshot(c, 4, revision, false);
        match case {
            0 => value["version"] = json!(5),
            1 => value["version"] = Value::Null,
            2 => {
                value.as_object_mut().unwrap().remove("version");
            }
            3 => value["conversation"] = json!(Uuid::new_v4()),
            4 => value["revision"] = json!(6),
            5 => value["paused"] = json!("false"),
            6 => value["paused"] = Value::Null,
            7 => {
                value.as_object_mut().unwrap().remove("paused");
            }
            _ => {}
        }
        insert_queue(&db, c, revision, &value);
        let before = row(&db, c);
        assert!(
            append(
                &mut db,
                c,
                Uuid::new_v4(),
                &terminal(c, TurnOutcome::Canceled)
            )
            .is_err()
        );
        assert_eq!(row(&db, c), before);
        unchanged_history(&db, c);
    }
    let c = conversation(&db);
    let value = snapshot(c, 4, i64::MAX, true);
    insert_queue(&db, c, i64::MAX, &value);
    append(
        &mut db,
        c,
        Uuid::new_v4(),
        &terminal(c, TurnOutcome::Interrupted),
    )
    .unwrap();
    assert_eq!(row(&db, c), (i64::MAX, value.to_string()));
}

#[test]
fn completed_terminal_leaves_queue_unchanged_and_absent_queue_retains_original_pause_default() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    for version in 1..=4 {
        let c = conversation(&db);
        let value = snapshot(c, version, 7, false);
        insert_queue(&db, c, 7, &value);
        append(
            &mut db,
            c,
            Uuid::new_v4(),
            &terminal(c, TurnOutcome::Completed),
        )
        .unwrap();
        assert_eq!(row(&db, c), (7, value.to_string()));
    }
    let c = conversation(&db);
    append(
        &mut db,
        c,
        Uuid::new_v4(),
        &terminal(c, TurnOutcome::Failed),
    )
    .unwrap();
    let (revision, raw) = row(&db, c);
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(revision, 1);
    assert_eq!(value["version"], 1);
    assert_eq!(value["paused"], true);
    assert_eq!(value["inputs"], json!([]));
    assert_eq!(value["preparing"], Value::Null);
}

#[test]
fn failed_215_registration_restores_deployed_trigger_checksum_and_queue_before_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("failed-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 213);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let old_checksums = checksums(&db);
    let original_trigger = trigger(&db);
    let c = conversation(&db);
    let value = snapshot(c, 4, 7, false);
    insert_queue(&db, c, 7, &value);
    db.execute_batch("create trigger reject_215 before insert on schema_migrations when new.version=215 begin select raise(abort,'synthetic migration rejection'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(latest(&db), 213);
    assert_eq!(trigger(&db), original_trigger);
    assert_eq!(checksums(&db), old_checksums);
    assert_eq!(row(&db, c), (7, value.to_string()));
    assert_eq!(
        db.query_row(
            "select count(*) from migration_checksums where version=215",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    assert_eq!(trigger(&db), original_trigger);
    db.execute_batch("drop trigger reject_215;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(latest(&db), latest_registered());
    assert_eq!(checksums(&db), old_checksums);
    append(
        &mut db,
        c,
        Uuid::new_v4(),
        &terminal(c, TurnOutcome::Failed),
    )
    .unwrap();
    assert_eq!(row(&db, c).0, 8);
}

#[test]
fn a_pause_update_trigger_failure_rolls_back_queue_history_and_original_batch_receipt() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let c = conversation(&db);
    let value = snapshot(c, 4, 7, false);
    insert_queue(&db, c, 7, &value);
    let before = row(&db, c);
    let event = terminal(c, TurnOutcome::Failed);
    let batch = Uuid::new_v4();
    db.execute_batch("create trigger reject_terminal_queue_update after update on agent_input_queues begin select raise(abort,'synthetic pause rejection'); end;").unwrap();
    assert!(append(&mut db, c, batch, &event).is_err());
    assert_eq!(row(&db, c), before);
    unchanged_history(&db, c);
    db.execute_batch("drop trigger reject_terminal_queue_update;")
        .unwrap();
    assert_eq!(append(&mut db, c, batch, &event).unwrap(), 1);
    assert_eq!(row(&db, c).0, 8);
}
