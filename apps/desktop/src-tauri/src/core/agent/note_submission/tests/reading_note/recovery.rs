use super::*;
use crate::agent::{note_submission::recovery as pages, reading_note_recovery as discovery};
use research_agent_core::protocol::{AgentEvent, EventPayload};

fn query(f: &Fixture, after: Option<Uuid>, limit: u8) -> discovery::Request {
    discovery::Request {
        conversation_id: f.conversation,
        reading_note_id: f.page,
        after,
        limit,
    }
}
#[tokio::test]
async fn recovery_pages_isolate_same_id_resources_and_keep_original_receipts_after_reopen() {
    let f = reading_fixture();
    let page = prepare(&f.store, &f.models, request(&f, Uuid::from_u128(1)))
        .await
        .unwrap();
    let mut saved = vec![];
    for index in 2..=4 {
        let mut input = reading_request(&f, Uuid::from_u128(index));
        input.text = format!("{index}{}", "文".repeat(300));
        saved.push(reading::prepare(&f.store, &f.models, input).await.unwrap());
    }
    {
        let db = f.store.connection().unwrap();
        db.execute("update note_pages set revision=revision+1 where id in (select page_id from reading_note_sources)",[]).unwrap();
        db.execute(
            "update reading_note_sources set quote_exact='Later quote'",
            [],
        )
        .unwrap();
        db.execute("update llm_provider_configs set enabled=0", [])
            .unwrap();
    }
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    let first = discovery::list(&reopened, query(&f, None, 2))
        .await
        .unwrap();
    assert_eq!(
        first.items.iter().map(|i| i.request_id).collect::<Vec<_>>(),
        vec![saved[0].request_id, saved[1].request_id]
    );
    assert_eq!(first.next_cursor, Some(saved[1].request_id));
    assert_eq!(first.items[0].message_excerpt.chars().count(), 200);
    let next = discovery::list(&reopened, query(&f, first.next_cursor, 2))
        .await
        .unwrap();
    assert_eq!(next.items.len(), 1);
    assert!(next.next_cursor.is_none());
    let page_list = pages::list(
        &reopened,
        pages::Request {
            conversation_id: f.conversation,
            page_id: f.page,
            after: None,
            limit: 64,
        },
    )
    .await
    .unwrap();
    assert_eq!(page_list.items.len(), 1);
    assert_eq!(page_list.items[0].request_id, page.request_id);
    assert_eq!(
        reading::restore(&reopened, reading_receipt(&f, first.items[0].request_id))
            .await
            .unwrap(),
        saved[0]
    );
    assert_eq!(count(&f), 4);
    assert_eq!(
        reopened
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let encoded = serde_json::to_value(first).unwrap();
    assert!(encoded.get("readingNoteId").is_some() && encoded.get("pageId").is_none());
    assert!(encoded["items"][0].get("input").is_none());
}
#[tokio::test]
async fn discovery_filters_started_and_wrong_source_identity_and_rechecks_live_access() {
    let f = reading_fixture();
    let saved = reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::from_u128(1)))
        .await
        .unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let mut wrong = saved.input.clone();
        wrong.context[0].source_id = "another-reading-source".into();
        insert_in(&tx, f.conversation, Uuid::from_u128(2), wrong).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        discovery::list(&f.store, query(&f, None, 64))
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
            turn_id: saved.turn_id,
            context_snapshot_id: saved.context_snapshot_id,
            model: saved.input.model.clone(),
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",params![f.conversation.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
    assert!(
        discovery::list(&f.store, query(&f, None, 64))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    for status in ["archived", "trashed"] {
        f.store
            .connection()
            .unwrap()
            .execute(
                "update note_pages set status=?1 where id in (select page_id from reading_note_sources)",
                [status],
            )
            .unwrap();
        assert!(matches!(
            discovery::list(&f.store, query(&f, None, 64)).await,
            Err(Error::Unbound)
        ));
    }
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set status='active' where id in (select page_id from reading_note_sources)",
            [],
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where resource_kind='readingNote'",
            [],
        )
        .unwrap();
    assert!(matches!(
        discovery::list(&f.store, query(&f, None, 64)).await,
        Err(Error::Unbound)
    ));
    // The same-ID Note Page binding does not restore reading permission.
    assert!(
        pages::list(
            &f.store,
            pages::Request {
                conversation_id: f.conversation,
                page_id: f.page,
                after: None,
                limit: 64
            }
        )
        .await
        .is_ok()
    );
}
#[tokio::test]
async fn reading_discovery_rejects_invalid_pagination_and_renderer_authority_fields() {
    let f = reading_fixture();
    for input in [
        query(&f, None, 0),
        query(&f, None, 65),
        query(&f, Some(Uuid::nil()), 1),
        discovery::Request {
            reading_note_id: Uuid::nil(),
            ..query(&f, None, 1)
        },
    ] {
        assert!(matches!(
            discovery::list(&f.store, input).await,
            Err(Error::InvalidInput)
        ));
    }
    let value = serde_json::json!({"conversationId":f.conversation,"readingNoteId":f.page,"after":null,"limit":20});
    assert!(serde_json::from_value::<discovery::Request>(value.clone()).is_ok());
    for key in ["start", "pageId", "tools"] {
        let mut wrong = value.clone();
        wrong[key] = serde_json::json!(true);
        assert!(serde_json::from_value::<discovery::Request>(wrong).is_err());
    }
}
