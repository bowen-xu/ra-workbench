use super::*;

#[tokio::test]
async fn native_request_metadata_advances_only_after_compaction_installation() {
    use crate::agent::repository::SqliteConversationCatalog;
    use research_agent_core::conversation::{
        BindingKind, ConversationBinding, ConversationForkRequest,
    };
    let mut fixture = Fixture::new().await;
    let parent = fixture.id;
    let connection = fixture.app.shared_agent_connection();
    {
        let db = connection.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('cache-note','Note','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','cache-note')", [parent.to_string()]).unwrap();
    }
    let (url, peer) = sequential_server_with_headers(vec![
        ok(&body()),
        ok(&events(&[opaque()], true)),
        ok(&body()),
        ok(&events(&[opaque()], true)),
        ok(&body()),
        ok(&body()),
    ])
    .await;
    let (lease, _epoch) = crate::agent::authentication::tests::synthetic_lease();
    let lease = Arc::new(lease);
    let mut turns = Vec::new();
    for (text, compact) in [
        ("first", false),
        ("/compact", true),
        ("next", false),
        ("/compact", true),
        ("last", false),
    ] {
        let record = fixture.record(text).await;
        turns.push(record.turn_id);
        let mut transport = Arc::try_unwrap(fixture.transport(&record, &url))
            .ok()
            .unwrap();
        transport.kind = ResponsesEndpointKind::ChatGpt;
        transport.auth = Authentication::SyntheticLease(lease.clone());
        let execution = fixture.execution(&record, Arc::new(transport));
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
        fixture
            .service
            .release_idle(ConversationId::from_uuid(fixture.id))
            .await
            .unwrap();
        fixture
            .service
            .open(ConversationId::from_uuid(fixture.id))
            .await
            .unwrap();
    }
    let child = Uuid::new_v4();
    SqliteConversationCatalog::new(connection)
        .fork_legacy_for_tests(ConversationForkRequest {
            parent_id: ConversationId::from_uuid(parent),
            child_id: ConversationId::from_uuid(child),
            through_turn_id: turns[4],
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "cache-note".into(),
            },
        })
        .await
        .unwrap();
    fixture.id = child;
    let record = fixture.record("child cache identity").await;
    let mut transport = Arc::try_unwrap(fixture.transport(&record, &url))
        .ok()
        .unwrap();
    transport.kind = ResponsesEndpointKind::ChatGpt;
    transport.auth = Authentication::SyntheticLease(lease);
    let execution = fixture.execution(&record, Arc::new(transport));
    assert_eq!(
        fixture
            .start(&record, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    fixture.id = parent;
    let (bodies, headers) = peer.await.unwrap();
    assert_eq!(bodies.len(), 6);
    assert_eq!(bodies[5]["prompt_cache_key"], child.to_string());
    assert_eq!(bodies[5]["client_metadata"]["thread_id"], child.to_string());
    assert_ne!(bodies[4]["prompt_cache_key"], bodies[5]["prompt_cache_key"]);
    for (index, number) in [0, 0, 1, 1, 2].into_iter().enumerate() {
        assert_eq!(bodies[index]["prompt_cache_key"], fixture.id.to_string());
        let window = format!("{}:{number}", fixture.id);
        assert_eq!(
            bodies[index]["client_metadata"]["thread_id"],
            fixture.id.to_string()
        );
        assert_eq!(
            bodies[index]["client_metadata"]["turn_id"],
            turns[index].to_string()
        );
        assert_eq!(
            bodies[index]["client_metadata"]["x-codex-window-id"],
            window
        );
        assert!(
            headers[index]
                .to_ascii_lowercase()
                .contains(&format!("x-codex-window-id: {window}"))
        );
    }
    let active = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        bodies[4]["client_metadata"]["x-codex-window-id"],
        active.window_id.to_string()
    );
}
