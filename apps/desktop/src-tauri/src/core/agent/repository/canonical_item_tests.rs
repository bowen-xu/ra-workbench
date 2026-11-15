use super::*;
use research_agent_core::{
    conversation::{ConversationId, HistoryQuery},
    protocol::*,
};
use rusqlite::Connection;
use uuid::Uuid;
#[path = "canonical_item_tests/correction.rs"]
mod correction;

fn item(id: ConversationId, seq: u64, item_id: Uuid, text: &str) -> AgentEvent {
    AgentEvent::new(
        id,
        seq,
        1,
        Uuid::from_u128(3),
        EventPayload::ItemAccepted {
            turn_id: Uuid::from_u128(2),
            item_id,
            kind: ItemKind::AssistantMessage,
            text: text.into(),
            phase: Some(AssistantMessagePhase::FinalAnswer),
            source: Some(CanonicalItemSource {
                attempt_id: Uuid::from_u128(4),
                ordinal: 0,
                provider_item_id: Some("native-1".into()),
                provider_data_json: Some("{\"version\":1,\"opaque\":\"fixture\"}".into()),
            }),
        },
    )
    .unwrap()
}

fn assert_canonical_storage(db: &Connection, id: ConversationId, slices: i64) {
    let counts: (i64, i64, i64) = db
        .query_row(
            "select sum(json_extract(payload_json,'$.payload.type')='storedTextSlice'),
            sum(json_extract(payload_json,'$.payload.type')='storedItemAccepted'
                and json_type(payload_json,'$.payload.text') is null),
            sum(json_extract(payload_json,'$.payload.type') in ('textDelta','itemAccepted'))
         from agent_journal_records where conversation_id=?1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(counts, (slices, 1, 0));
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

fn migration_checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=205 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn canonical_item_upgrade_reopen_and_atomic_failure_preserve_history() {
    let path = std::env::temp_dir().join(format!("agent-canonical-{}.sqlite", Uuid::new_v4()));
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let item_id = Uuid::new_v4();
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 204);
    create_conversation(&mut db, id, "canonical fixture").unwrap();
    let legacy = [
        EventPayload::ItemStarted {
            turn_id: Uuid::from_u128(2),
            item_id,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::TextDelta {
            turn_id: Uuid::from_u128(2),
            item_id,
            text: "accepted🙂\0tail".into(),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, payload)| {
        AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
    })
    .collect::<Vec<_>>();
    append_batch(&mut db, id, Uuid::new_v4(), 0, &legacy).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let accepted = item(id, 3, item_id, "accepted🙂\0tail");
    db.execute_batch("create trigger fail_canonical before update on agent_messages begin select raise(abort,'fixture'); end;").unwrap();
    let batch = Uuid::new_v4();
    assert_eq!(
        append_batch(&mut db, id, batch, 2, std::slice::from_ref(&accepted)),
        Err(JournalError::Unavailable)
    );
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
            .unwrap()
            .items,
        legacy
    );
    assert!(
        !db.query_row("select completed from agent_messages", [], |row| row
            .get::<_, bool>(0))
            .unwrap()
    );
    db.execute_batch("drop trigger fail_canonical").unwrap();
    for kind in ["storedTextSlice", "storedItemAccepted"] {
        db.execute_batch(&format!("create trigger fail_canonical_storage before update on agent_journal_records
            when json_extract(new.payload_json,'$.payload.type')='{kind}' begin select raise(abort,'fixture'); end;")).unwrap();
        assert_eq!(
            append_batch(&mut db, id, batch, 2, std::slice::from_ref(&accepted)),
            Err(JournalError::Unavailable)
        );
        assert_eq!(
            read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
                .unwrap()
                .items,
            legacy
        );
        assert!(
            !db.query_row("select completed from agent_messages", [], |row| row
                .get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(
            db.query_row(
                "select count(*) from agent_journal_batches where batch_id=?1",
                [batch.to_string()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger fail_canonical_storage")
            .unwrap();
    }
    assert_eq!(
        append_batch(&mut db, id, batch, 2, std::slice::from_ref(&accepted)),
        Ok(3)
    );
    assert_canonical_storage(&db, id, 1);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_canonical_storage(&db, id, 1);
    assert_eq!(
        append_batch(&mut db, id, batch, 2, std::slice::from_ref(&accepted)),
        Ok(3)
    );
    let history =
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap()).unwrap();
    assert_eq!(history.items, [legacy, vec![accepted]].concat());
    assert_eq!(
        db.query_row("select body,completed from agent_messages", [], |row| Ok((
            row.get::<_, String>(0)?,
            row.get::<_, bool>(1)?
        )))
        .unwrap(),
        ("accepted🙂\0tail".into(), true)
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn standalone_canonical_item_populates_the_existing_search_projection() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "fixture").unwrap();
    let event = item(id, 1, Uuid::new_v4(), "native answer");
    append_batch(&mut db, id, Uuid::new_v4(), 0, &[event]).unwrap();
    assert_canonical_storage(&db, id, 0);
    assert_eq!(
        db.query_row(
            "select body,completed from agent_search_messages",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        )
        .unwrap(),
        ("native answer".into(), true)
    );
}

#[test]
fn deployed_205_canonical_items_upgrade_without_changing_events_or_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("canonical.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 205);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let checksums = migration_checksums(&db);
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "canonical fixture").unwrap();
    let item_id = Uuid::new_v4();
    let mut events = vec![EventPayload::ItemStarted {
        turn_id: Uuid::from_u128(2),
        item_id,
        kind: ItemKind::AssistantMessage,
    }];
    for text in ["accepted🙂", "\0tail\n"] {
        events.push(EventPayload::TextDelta {
            turn_id: Uuid::from_u128(2),
            item_id,
            text: text.into(),
        });
    }
    let mut events = events
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
        })
        .collect::<Vec<_>>();
    events.push(item(id, 4, item_id, "accepted🙂\0tail\n完整"));
    let batch = Uuid::new_v4();
    append_batch(&mut db, id, batch, 0, &events).unwrap();
    db.execute("insert into agent_context_usage_observations(conversation_id,model_json,window_key,rollback_epoch,source_ordinal,sample_json) values (?1,'{}','',0,1,'{}')",[id.as_uuid().to_string()]).unwrap();
    let active = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, active, "active fixture").unwrap();
    let active_item = Uuid::new_v4();
    let pending = [
        EventPayload::ItemStarted {
            turn_id: Uuid::from_u128(2),
            item_id: active_item,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::TextDelta {
            turn_id: Uuid::from_u128(2),
            item_id: active_item,
            text: "active🙂\0tail".into(),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, payload)| {
        AgentEvent::new(active, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
    })
    .collect::<Vec<_>>();
    append_batch(&mut db, active, Uuid::new_v4(), 0, &pending).unwrap();
    let search: (String, bool) = db
        .query_row(
            "select body,completed from agent_search_messages where conversation_id=?1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(db.query_row("select count(*) from agent_journal_records where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='itemAccepted'",[id.as_uuid().to_string()],|row|row.get::<_,i64>(0)).unwrap(),1);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(migration_checksums(&db), checksums);
    assert_canonical_storage(&db, id, 2);
    let restored = read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
        .unwrap()
        .items;
    assert_eq!(restored, events);
    assert_eq!(
        serde_json::to_string(&restored).unwrap(),
        serde_json::to_string(&events).unwrap()
    );
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(active, 0, 500).unwrap())
            .unwrap()
            .items,
        pending
    );
    assert_eq!(
        db.query_row(
            "select body,completed from agent_search_messages where conversation_id=?1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        )
        .unwrap(),
        search
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_usage_observations",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(append_batch(&mut db, id, batch, 0, &events), Ok(4));
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 2, 1).unwrap())
            .unwrap()
            .items,
        events[2..3]
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_canonical_storage(&db, id, 2);
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
            .unwrap()
            .items,
        events
    );
    assert_eq!(append_batch(&mut db, id, batch, 0, &events), Ok(4));
}

#[test]
fn deployed_205_upgrade_failure_rolls_back_rewritten_rows_and_schema() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 205);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let checksums = migration_checksums(&db);
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "fixture").unwrap();
    let accepted = item(id, 1, Uuid::new_v4(), "升级🙂\0tail");
    append_batch(
        &mut db,
        id,
        Uuid::new_v4(),
        0,
        std::slice::from_ref(&accepted),
    )
    .unwrap();
    db.execute_batch("create trigger fail_canonical_upgrade before update on agent_journal_records when json_extract(new.payload_json,'$.payload.type')='storedItemAccepted' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(migration_checksums(&db), checksums);
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        205
    );
    assert_eq!(
        db.query_row(
            "select json_extract(payload_json,'$.payload.type') from agent_journal_records",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "itemAccepted"
    );
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
            .unwrap()
            .items,
        vec![accepted.clone()]
    );
    db.execute_batch("drop trigger fail_canonical_upgrade")
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_canonical_storage(&db, id, 0);
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
            .unwrap()
            .items,
        vec![accepted]
    );
}

#[test]
fn canonical_completion_compacts_many_unicode_fragments_at_original_cursors() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "many fragments").unwrap();
    let item_id = Uuid::new_v4();
    let mut payloads = vec![EventPayload::ItemStarted {
        turn_id: Uuid::from_u128(2),
        item_id,
        kind: ItemKind::AssistantMessage,
    }];
    let mut body = String::new();
    for index in 0..1200 {
        let text = format!("片段{index}🙂\0");
        body.push_str(&text);
        payloads.push(EventPayload::TextDelta {
            turn_id: Uuid::from_u128(2),
            item_id,
            text,
        });
    }
    let mut events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
        })
        .collect::<Vec<_>>();
    events.push(item(id, events.len() as u64 + 1, item_id, &body));
    let mut count = 0;
    for chunk in events.chunks(500) {
        count = append_batch(&mut db, id, Uuid::new_v4(), count, chunk).unwrap();
    }
    assert_eq!(count, 1202);
    assert_canonical_storage(&db, id, 1200);
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 1199, 10).unwrap())
            .unwrap()
            .items,
        events[1199..]
    );
    assert_eq!(
        db.query_row("select body from agent_search_messages", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        body
    );
}

#[test]
fn mismatching_old_delta_is_never_reinterpreted_as_canonical_text() {
    for version in [205, 206] {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, version);
        let id = ConversationId::from_uuid(Uuid::new_v4());
        create_conversation(&mut db, id, "fixture").unwrap();
        let item_id = Uuid::new_v4();
        let mut events = [
            EventPayload::ItemStarted {
                turn_id: Uuid::from_u128(2),
                item_id,
                kind: ItemKind::AssistantMessage,
            },
            EventPayload::TextDelta {
                turn_id: Uuid::from_u128(2),
                item_id,
                text: "旧🙂\0text".into(),
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
        })
        .collect::<Vec<_>>();
        events.push(item(id, 3, item_id, "不同🙂\0final"));
        append_batch(&mut db, id, Uuid::new_v4(), 0, &events).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(
            read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap())
                .unwrap()
                .items,
            events
        );
        assert_eq!(db.query_row("select json_extract(payload_json,'$.payload.type') from agent_journal_records where ordinal=1",[],|row|row.get::<_,String>(0)).unwrap(),"textDelta");
        assert_eq!(db.query_row("select json_extract(payload_json,'$.payload.type') from agent_journal_records where ordinal=2",[],|row|row.get::<_,String>(0)).unwrap(),"storedItemAccepted");
    }
}
