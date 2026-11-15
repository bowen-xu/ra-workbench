use super::*;
use crate::agent::writing_application_commands::discovery;
use serde_json::{Value, json};

async fn page(
    f: &crate::agent::writing_submission::tests::Fixture,
    after: Option<u64>,
    limit: u8,
) -> Value {
    serde_json::to_value(
        discovery::list(
            &f.store,
            discovery::ListRequest {
                manuscript_id: f.manuscript,
                after,
                limit,
            },
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn application_discovery_retains_partial_journal_after_chat_deletion_and_database_reopen() {
    let f = deleted_chat::full_fixture();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing_with_extra(&f, false, true).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let read = || {
        service.read_proposal_application(
            original.conversation,
            original.manuscript,
            original.proposal,
            original.operation,
        )
    };
    let journal = read().await.unwrap();
    let initial = page(&f, None, 64).await;
    assert_eq!(
        initial["items"][0]["application"]["scope"]["operationId"],
        json!(original.operation)
    );
    assert_eq!(
        initial["items"][0]["application"]["files"][0]["phase"],
        "acknowledged"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert_eq!(page(&f, None, 64).await, initial);
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert_eq!(page(&f, None, 64).await, initial);
    f.store.connection().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![f.conversation.to_string(),f.manuscript.to_string()]).unwrap();
    deleted_chat::delete_chat(&f).await;
    assert_eq!(page(&f, None, 64).await, initial);
    assert_eq!(read().await.unwrap(), journal);
    let database = f.root.path().join("pending-application.sqlite");
    f.store
        .connection()
        .unwrap()
        .execute("vacuum into ?1", [database.to_str().unwrap()])
        .unwrap();
    let reopened = AppStore::for_test(
        Connection::open(database).unwrap(),
        f.store.content_directory().to_path_buf(),
    );
    let restored = discovery::list(
        &reopened,
        discovery::ListRequest {
            manuscript_id: f.manuscript,
            after: None,
            limit: 64,
        },
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), initial);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    assert!(!source(&f).join("other.tex").exists());
    // Discovery did not acquire new execution or first-application authority.
    assert!(
        service
            .begin_proposal_application(
                f.conversation,
                original.proposal,
                Uuid::new_v4(),
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn application_discovery_paginates_only_original_unresolved_operations() {
    let f = fixture_with_objects();
    let first = existing(&f, false).await;
    let second = existing(&f, true).await;
    let initial = page(&f, None, 1).await;
    let cursor = initial["nextCursor"].as_u64().unwrap();
    assert_eq!(
        initial["items"][0]["application"]["scope"]["operationId"],
        json!(first.operation)
    );
    let last = page(&f, Some(cursor), 1).await;
    assert_eq!(
        last["items"][0]["application"]["scope"]["operationId"],
        json!(second.operation)
    );
    assert!(last["items"][0]["id"].as_u64().unwrap() > cursor);
    assert_eq!(last["nextCursor"], Value::Null);
    let service = WritingSubmissionService::from_app_store(&f.store);
    service
        .apply_disk_proposal_file(first, "main.tex".into())
        .await
        .unwrap();
    service
        .prepare_proposal_undo(
            first,
            Uuid::new_v4(),
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        page(&f, None, 64).await["items"][0]["application"]["scope"]["operationId"],
        json!(second.operation)
    );
}

#[tokio::test]
async fn application_discovery_rejects_invalid_scope_cursor_and_corrupt_frozen_inventory() {
    let f = fixture_with_objects();
    let original = existing(&f, false).await;
    for (manuscript_id, after, limit) in [
        (Uuid::nil(), None, 1),
        (Uuid::new_v4(), None, 1),
        (f.manuscript, None, 0),
        (f.manuscript, None, 65),
        (f.manuscript, Some(0), 1),
        (f.manuscript, Some(9_007_199_254_740_992), 1),
    ] {
        assert!(
            discovery::list(
                &f.store,
                discovery::ListRequest {
                    manuscript_id,
                    after,
                    limit
                }
            )
            .await
            .is_err()
        );
    }
    assert!(serde_json::from_value::<discovery::ListRequest>(json!({"manuscriptId":f.manuscript,"after":null,"limit":1,"conversationId":f.conversation})).is_err());
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger agent_change_set_baseline_immutable")
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_change_sets set baseline_json='{}' where id=?1",
            [original.proposal.to_string()],
        )
        .unwrap();
    assert!(
        discovery::list(
            &f.store,
            discovery::ListRequest {
                manuscript_id: f.manuscript,
                after: None,
                limit: 64
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}
