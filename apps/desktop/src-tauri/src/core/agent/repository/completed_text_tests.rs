use super::*;
use research_agent_core::{
    conversation::{ConversationId, HistoryQuery},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use rusqlite::{Connection, params};
use uuid::Uuid;

fn events(id: ConversationId) -> Vec<AgentEvent> {
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let request = Uuid::new_v4();
    [
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "你好🙂".into(),
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "\0tail\n".into(),
        },
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: item,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(i, payload)| AgentEvent::new(id, i as u64 + 1, i as u64 + 1, request, payload).unwrap())
    .collect()
}
fn read(db: &mut Connection, id: ConversationId) -> Vec<AgentEvent> {
    read_history(db, HistoryQuery::new(id, 0, 500).unwrap())
        .unwrap()
        .items
}
fn assert_compacted(db: &Connection, id: ConversationId) {
    let body: (String, bool) = db
        .query_row(
            "select body,completed from agent_messages where conversation_id=?1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(body, ("你好🙂\0tail\n".into(), true));
    let count:i64=db.query_row("select count(*) from agent_journal_records where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='textDelta'",[id.as_uuid().to_string()],|row|row.get(0)).unwrap();
    assert_eq!(count, 0);
    let slices:i64=db.query_row("select count(*) from agent_journal_records where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='storedTextSlice' and json_type(payload_json,'$.payload.text') is null",[id.as_uuid().to_string()],|row|row.get(0)).unwrap();
    assert_eq!(slices, 2);
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}
#[test]
fn completion_removes_delta_text_atomically_and_preserves_history_cursors_and_receipts() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "Fixture").unwrap();
    let events = events(id);
    let batch = Uuid::new_v4();
    append_batch(&mut db, id, batch, 0, &events[..3]).unwrap();
    db.execute("insert into agent_context_usage_observations(conversation_id,model_json,window_key,rollback_epoch,source_ordinal,sample_json) values (?1,'{}','',0,1,'{}')",[id.as_uuid().to_string()]).unwrap();
    assert_eq!(read(&mut db, id), events[..3]);
    assert_eq!(db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='textDelta'",[],|row|row.get::<_,i64>(0)).unwrap(),2);
    db.execute_batch("create trigger fail_completion before update on agent_journal_records when json_extract(new.payload_json,'$.payload.type')='storedTextSlice' begin select raise(abort,'fixture'); end;").unwrap();
    let completion = Uuid::new_v4();
    assert_eq!(
        append_batch(&mut db, id, completion, 3, &events[3..]),
        Err(JournalError::Unavailable)
    );
    assert_eq!(read(&mut db, id), events[..3]);
    db.execute_batch("drop trigger fail_completion").unwrap();
    assert_eq!(
        append_batch(&mut db, id, completion, 3, &events[3..]),
        Ok(4)
    );
    assert_compacted(&db, id);
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_usage_observations",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(read(&mut db, id), events);
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 2, 1).unwrap())
            .unwrap()
            .items,
        events[2..3]
    );
    assert_eq!(append_batch(&mut db, id, batch, 0, &events[..3]), Ok(3));
    assert_eq!(
        append_batch(&mut db, id, completion, 3, &events[3..]),
        Ok(4)
    );
}
#[test]
fn deployed_journal_upgrade_reopens_with_complete_text_and_leaves_active_deltas_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 174);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let old_checksums: Vec<(i64, String)> = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let active = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "Complete").unwrap();
    create_conversation(&mut db, active, "Active").unwrap();
    let completed = events(id);
    let pending = events(active);
    append_batch(&mut db, id, Uuid::new_v4(), 0, &completed).unwrap();
    append_batch(&mut db, active, Uuid::new_v4(), 0, &pending[..3]).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let retained: Vec<(i64, String)> = db
        .prepare(
            "select version,sha256 from migration_checksums where version<=174 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(retained, old_checksums);
    assert_compacted(&db, id);
    assert_eq!(read(&mut db, id), completed);
    assert_eq!(read(&mut db, active), pending[..3]);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_compacted(&db, id);
    assert_eq!(read(&mut db, id), completed);
    append_batch(&mut db, active, Uuid::new_v4(), 3, &pending[3..]).unwrap();
    assert_compacted(&db, active);
    db.execute(
        "delete from agent_conversations where id=?1",
        params![id.as_uuid().to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_messages where conversation_id=?1",
            [id.as_uuid().to_string()],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn thousands_of_deltas_compact_in_one_pass_and_resume_at_the_original_cursor() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "Long reply").unwrap();
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let request = Uuid::new_v4();
    let mut payloads = vec![EventPayload::ItemStarted {
        turn_id: turn,
        item_id: item,
        kind: ItemKind::AssistantMessage,
    }];
    let mut body = String::new();
    for index in 0..3000 {
        let text = format!("片段{index}🙂");
        body.push_str(&text);
        payloads.push(EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text,
        });
    }
    payloads.push(EventPayload::ItemCompleted {
        turn_id: turn,
        item_id: item,
    });
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| AgentEvent::new(id, index as u64 + 1, 1, request, payload).unwrap())
        .collect();
    let mut count = 0;
    for batch in events.chunks(500) {
        count = append_batch(&mut db, id, Uuid::new_v4(), count, batch).unwrap();
    }
    assert_eq!(
        db.query_row("select body from agent_messages", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        body
    );
    assert_eq!(db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='textDelta'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 2999, 10).unwrap())
            .unwrap()
            .items,
        events[2999..]
    );
}

#[tokio::test]
async fn completed_text_can_be_exported_searched_and_forked_after_compaction() {
    use research_agent_core::conversation::{
        BindingKind, CatalogQuery, ConversationBinding, ConversationForkRequest,
        ConversationSearch, ConversationSearchQuery,
    };
    use research_agent_core::protocol::{ModelIdentity, TaskKind, TurnOutcome};
    use std::sync::{Arc, Mutex};
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let child = ConversationId::from_uuid(Uuid::new_v4());
    create_conversation(&mut db, id, "Fixture").unwrap();
    let text_events = events(id);
    let EventPayload::ItemStarted { turn_id, .. } = text_events[0].payload() else {
        unreachable!()
    };
    let turn = *turn_id;
    let request = text_events[0].request_id();
    let mut payloads = vec![EventPayload::TurnStarted {
        task_kind: TaskKind::Regular,
        turn_id: turn,
        context_snapshot_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
    }];
    payloads.extend(text_events.iter().map(|event| event.payload().clone()));
    payloads.push(EventPayload::TurnFinished {
        turn_id: turn,
        outcome: TurnOutcome::Completed,
    });
    let history: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| AgentEvent::new(id, index as u64 + 1, 1, request, payload).unwrap())
        .collect();
    append_batch(&mut db, id, Uuid::new_v4(), 0, &history).unwrap();
    let note = Uuid::new_v4().to_string();
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values (?1,'Fixture','now','now')",
        [&note],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note',?2)",params![id.as_uuid().to_string(),note]).unwrap();
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let exported = catalog.export_snapshot(id).await.unwrap();
    assert_eq!(exported.turns[0].items[0].text, "你好🙂\0tail\n");
    let found = catalog
        .search(ConversationSearchQuery {
            term: "你好".into(),
            scope: CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 10,
                before: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(found.hits.len(), 1);
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: child,
            parent_id: id,
            through_turn_id: turn,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: note,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        catalog.export_snapshot(child).await.unwrap().turns[0].items[0].text,
        "你好🙂\0tail\n"
    );
    assert_compacted(&db.lock().unwrap(), child);
}
