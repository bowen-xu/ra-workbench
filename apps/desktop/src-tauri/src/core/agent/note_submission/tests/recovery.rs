use super::*;
use crate::agent::note_submission::recovery::{Request, list};
use research_agent_core::protocol::{AgentEvent, EventPayload};

fn query(f: &Fixture, after: Option<Uuid>, limit: u8) -> Request {
    Request {
        conversation_id: f.conversation,
        page_id: f.page,
        after,
        limit,
    }
}

#[tokio::test]
async fn discovers_original_receipts_after_reopen_without_auth_or_current_note_refresh() {
    let f = fixture(true);
    let mut records = Vec::new();
    for index in 1..=3 {
        let mut input = request(&f, Uuid::from_u128(index));
        input.text = format!("{index}{}", "文".repeat(300));
        records.push(prepare(&f.store, &f.models, input).await.unwrap());
    }
    {
        let db = f.store.connection().unwrap();
        db.execute("update note_pages set revision=revision+1", [])
            .unwrap();
        db.execute("update llm_provider_configs set enabled=0", [])
            .unwrap();
    }
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    let first = list(&reopened, query(&f, None, 2)).await.unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.next_cursor, Some(records[1].request_id));
    assert_eq!(first.items[0].turn_id, records[0].turn_id);
    assert_eq!(first.items[0].message_excerpt.chars().count(), 200);
    let next = list(&reopened, query(&f, first.next_cursor, 2))
        .await
        .unwrap();
    assert_eq!(next.items.len(), 1);
    assert!(next.next_cursor.is_none());
    assert_eq!(
        restore(&reopened, receipt(&f, first.items[0].request_id))
            .await
            .unwrap(),
        records[0]
    );
    assert_eq!(count(&f), 3);
    let events: i64 = reopened
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn filters_other_sources_and_started_turns_and_rechecks_binding() {
    let f = fixture(true);
    let record = prepare(&f.store, &f.models, request(&f, Uuid::from_u128(1)))
        .await
        .unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let mut different_page = record.input.clone();
        different_page.context[0].resource_id = Uuid::new_v4().to_string();
        insert_in(&tx, f.conversation, Uuid::from_u128(2), different_page).unwrap();
        let mut different_host = record.input.clone();
        different_host.context[0].kind = ContextSourceKind::Manuscript;
        insert_in(&tx, f.conversation, Uuid::from_u128(3), different_host).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        list(&f.store, query(&f, None, 64))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.conversation),
        1,
        0,
        Uuid::new_v4(),
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",params![f.conversation.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
    assert!(
        list(&f.store, query(&f, None, 64))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let mut wrong = query(&f, None, 64);
    wrong.page_id = Uuid::new_v4();
    assert!(matches!(list(&f.store, wrong).await, Err(Error::Unbound)));
    f.store
        .connection()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(matches!(
        list(&f.store, query(&f, None, 64)).await,
        Err(Error::Unbound)
    ));
}

#[tokio::test]
async fn rejects_invalid_cursor_and_limits_before_reading() {
    let f = fixture(true);
    for request in [
        query(&f, None, 0),
        query(&f, None, 65),
        query(&f, Some(Uuid::nil()), 1),
        Request {
            page_id: Uuid::nil(),
            ..query(&f, None, 1)
        },
    ] {
        assert!(matches!(
            list(&f.store, request).await,
            Err(Error::InvalidInput)
        ));
    }
    let mut value = serde_json::json!({"conversationId":f.conversation,"pageId":f.page,"after":null,"limit":20});
    assert!(serde_json::from_value::<Request>(value.clone()).is_ok());
    value["start"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Request>(value).is_err());
}
