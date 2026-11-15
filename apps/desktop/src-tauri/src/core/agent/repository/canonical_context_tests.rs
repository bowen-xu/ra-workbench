use super::*;
use research_agent_core::context::checkpoint::{CanonicalContext, StoredContent, StoredMessage};

fn through_133(db: &mut Connection) {
    for &(version, sql) in crate::storage_migrations::MIGRATIONS {
        if !(127..=133).contains(&version) {
            continue;
        }
        let rebuild =
            crate::storage_migrations::MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
        if rebuild {
            db.execute_batch("pragma foreign_keys=off").unwrap();
        }
        let tx = db.transaction().unwrap();
        tx.execute_batch(sql).unwrap();
        tx.execute(
            "insert into schema_migrations(version,applied_at) values(?1,'now')",
            [version],
        )
        .unwrap();
        tx.commit().unwrap();
        if rebuild {
            db.execute_batch("pragma foreign_keys=on").unwrap();
        }
    }
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
}

fn through_126(db: &mut Connection, saved: &ContextCheckpoint) {
    seed_responses_binding_125(db, saved);
    db.execute_batch("pragma foreign_keys=off").unwrap();
    {
        let tx = db.transaction().unwrap();
        tx.execute_batch(include_str!(
            "../../../../migrations/0126_agent_empty_context_windows.sql"
        ))
        .unwrap();
        tx.execute(
            "insert into schema_migrations(version,applied_at) values(126,'now')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    db.execute_batch("pragma foreign_keys=on").unwrap();
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
}

#[test]
fn schema_two_upgrade_preserves_old_window_and_installs_an_independent_canonical_snapshot() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    through_126(&mut db, &saved);
    let encoded: String = db
        .query_row(
            "select checkpoint_json from agent_context_checkpoints where id=?1",
            [saved.window_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row(
            "select checkpoint_json from agent_context_checkpoints where id=?1",
            [saved.window_id.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        encoded
    );
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    let mut next = saved.clone();
    next.schema_version = 2;
    next.window_id = Uuid::new_v4();
    next.previous_window_id = Some(saved.window_id);
    next.phase = CompactionPhase::MidTurn;
    next.canonical_context = Some(CanonicalContext {
        before_retained: 0,
        messages: vec![StoredMessage::User {
            source_item_id: None,
            content: vec![StoredContent::Text {
                text: "frozen canonical context".into(),
            }],
        }],
    });
    install(&mut db, &next).unwrap();
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == next);
    assert_eq!(
        db.query_row(
            "select schema_version from agent_context_checkpoints where id=?1",
            [next.window_id.to_string()],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "select window_id from agent_responses_window_bindings where turn_id=?1",
            [saved.turn_id.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        saved.window_id.to_string()
    );
    assert!(
        db.execute(
            "update agent_context_checkpoints set schema_version=1 where id=?1",
            [next.window_id.to_string()]
        )
        .is_err()
    );
    assert!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}

#[test]
fn column_and_json_schema_mismatch_is_rejected_on_read() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,schema_version,checkpoint_json) values(?1,?2,?3,1,2,?4)",
        params![saved.window_id.to_string(), saved.conversation_id.to_string(), saved.turn_id.to_string(), serde_json::to_string(&saved).unwrap()]).unwrap();
    db.execute(
        "insert into agent_active_context_windows(conversation_id,window_id) values(?1,?2)",
        params![
            saved.conversation_id.to_string(),
            saved.window_id.to_string()
        ],
    )
    .unwrap();
    assert!(matches!(
        read_active(&db, saved.conversation_id),
        Err(Error::Invalid)
    ));
}

#[test]
fn schema_two_upgrade_failure_restores_old_table_and_foreign_key_enforcement() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    through_126(&mut db, &saved);
    db.execute_batch("pragma foreign_keys=off").unwrap();
    db.execute(
        "update agent_responses_window_bindings set window_id='missing-window'",
        [],
    )
    .unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=127",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_master where name='agent_context_checkpoints_0127'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert!(
        db.execute(
            "update agent_context_checkpoints set schema_version=2 where id=?1",
            [saved.window_id.to_string()]
        )
        .is_err()
    );
    assert!(db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,schema_version,checkpoint_json) values(?1,?2,?3,1,2,'{}')",
        params![Uuid::new_v4().to_string(), saved.conversation_id.to_string(), saved.turn_id.to_string()]).is_err());
}

#[test]
fn schema_three_upgrade_preserves_bytes_bindings_and_pending_hooks_then_stores_developer_role() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    through_126(&mut db, &saved);
    through_133(&mut db);
    let pending = saved.window_id.to_string();
    db.execute(
        "insert into agent_compact_session_start(conversation_id,window_id) values(?1,?2)",
        params![saved.conversation_id.to_string(), pending],
    )
    .unwrap();
    let original: String = db
        .query_row(
            "select checkpoint_json from agent_context_checkpoints where id=?1",
            [&pending],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let after: String = db
        .query_row(
            "select checkpoint_json from agent_context_checkpoints where id=?1",
            [&pending],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, after);
    assert_eq!(
        db.query_row(
            "select window_id from agent_compact_session_start where conversation_id=?1",
            [saved.conversation_id.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        pending
    );
    let mut next = saved.clone();
    next.window_id = Uuid::new_v4();
    next.previous_window_id = Some(saved.window_id);
    next.schema_version = 3;
    next.binding.kind = CompactionKind::ResetWindow;
    next.output = CompactionOutput::ResetWindow;
    next.canonical_context = Some(CanonicalContext {
        before_retained: 0,
        messages: vec![StoredMessage::Developer {
            text: "trusted guidance".into(),
        }],
    });
    install(&mut db, &next).unwrap();
    let restored = read_active(&db, saved.conversation_id).unwrap().unwrap();
    assert!(restored == next);
    assert!(
        matches!(&restored.restore_reset_window(&|_| None).unwrap()[0], research_agent_core::model::ModelMessage::Developer(text) if text == "trusted guidance")
    );
    assert!(
        db.execute(
            "update agent_context_checkpoints set schema_version=2 where id=?1",
            [next.window_id.to_string()]
        )
        .is_err()
    );
    assert!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}

#[test]
fn schema_three_upgrade_failure_rolls_back_table_and_restores_foreign_key_enforcement() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    through_126(&mut db, &saved);
    through_133(&mut db);
    db.execute_batch("pragma foreign_keys=off").unwrap();
    db.execute("insert into agent_compact_session_start(conversation_id,window_id) values(?1,'missing-window')", [saved.conversation_id.to_string()]).unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=134",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_master where name='agent_context_checkpoints_0134'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
}
