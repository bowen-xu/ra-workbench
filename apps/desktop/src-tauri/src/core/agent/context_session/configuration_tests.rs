use super::*;

fn fixture(archived: bool) -> (Connection, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title,archived) values(?1,'Migrated',?2)",
        params![conversation.to_string(), archived],
    )
    .unwrap();
    for paper in ["paper", "other"] {
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'document',?1,'now','now')",[paper]).unwrap();
    }
    (db, conversation)
}

fn selected() -> Value {
    json!({"schemaVersion":1,"persistent":[{"resource":{"kind":"paperMetadata","resourceId":"paper"},"label":"Persisted metadata"}],"temporary":[{"resource":{"kind":"paperMetadata","resourceId":"other"},"label":"One turn metadata"}]})
}

#[test]
fn native_explicit_empty_wins_over_legacy_choices_without_validation() {
    let (db, conversation) = fixture(false);
    seed_in(
        &db,
        conversation,
        &[],
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let original = read_in(&db, conversation).unwrap().unwrap();
    import_legacy_in(
        &db,
        conversation,
        &selected(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    import_legacy_in(
        &db,
        conversation,
        &json!({"invalid":"old cache"}),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let saved = read_in(&db, conversation).unwrap().unwrap();
    assert_eq!(saved.revision, original.revision);
    assert_eq!(saved.state, original.state);
}

#[test]
fn archived_configuration_recovers_temporary_selections_without_write_authority() {
    let (db, conversation) = fixture(true);
    let state = selected();
    import_legacy_in(
        &db,
        conversation,
        &state,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let saved = read_in(&db, conversation).unwrap().unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.state, state);
    assert_eq!(saved.state["persistent"].as_array().unwrap().len(), 1);
    assert_eq!(saved.state["temporary"].as_array().unwrap().len(), 1);
    assert_eq!(
        check(&db, conversation, true),
        Err(Error::ConversationUnavailable)
    );
    assert_eq!(
        seed_in(
            &db,
            conversation,
            &[],
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::ConversationUnavailable)
    );
}

#[test]
fn deleted_conversations_are_skipped_without_creating_configuration() {
    let (db, conversation) = fixture(false);
    db.execute(
        "delete from agent_conversations where id=?1",
        [conversation.to_string()],
    )
    .unwrap();
    import_legacy_in(
        &db,
        conversation,
        &selected(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from agent_context_configurations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn invalid_foreign_scope_is_reported_and_same_client_selection_can_be_repaired() {
    let (db, conversation) = fixture(false);
    let state = json!({"schemaVersion":1,"persistent":[{"resource":{"kind":"paperGroup","resourceId":"paper","parts":[{"resource":{"kind":"paperMetadata","resourceId":"other"},"label":"Foreign metadata","selected":true,"access":"readOnly"}]},"label":"Paper"}],"temporary":[]});
    assert_eq!(
        import_legacy_in(
            &db,
            conversation,
            &state,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::InvalidInput)
    );
    assert!(read_in(&db, conversation).unwrap().is_none());
    assert_eq!(
        state["persistent"][0]["resource"]["parts"][0]["resource"]["resourceId"],
        "other"
    );
    let mut repaired = state;
    repaired["persistent"][0]["resource"]["parts"][0]["resource"]["resourceId"] = json!("paper");
    import_legacy_in(
        &db,
        conversation,
        &repaired,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(read_in(&db, conversation).unwrap().unwrap().state, repaired);
}
