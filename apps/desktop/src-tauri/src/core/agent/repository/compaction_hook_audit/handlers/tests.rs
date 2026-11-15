use super::*;

#[test]
fn migration_141_preserves_legacy_audit_without_inventing_handler_evidence() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 140);
    let conversation = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        research_agent_core::conversation::ConversationId::from_uuid(conversation),
        "legacy audit",
    )
    .unwrap();
    let operation = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let model = r#"{"providerId":"legacy","modelId":"model"}"#;
    db.execute("insert into agent_compaction_hook_audit(conversation_id,operation_id,turn_id,stage,phase,model_json,notification,recorded_at_ms) values(?1,?2,?3,'pre_compact','manual',?4,'started',17)", params![conversation.to_string(),operation.to_string(),turn.to_string(),model]).unwrap();
    let original = db.last_insert_rowid();
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let saved: (i64, String, i64) = db
        .query_row(
            "select seq,model_json,recorded_at_ms from agent_compaction_hook_audit",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(saved, (original, model.into(), 17));
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compaction_hook_handlers",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}
