use super::*;
use crate::agent::writing_application_commands::undo::discovery;
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
async fn discovery_reads_durable_pending_phases_without_backfill_or_execution() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(page(&f, None, 64).await["items"], json!([]));
    let undo = Uuid::new_v4();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    service
        .prepare_proposal_undo(original, undo, request())
        .await
        .unwrap();
    let _reopened = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let prepared = page(&f, None, 1).await;
    let database_path = f.root.path().join("pending-undo.sqlite");
    f.store
        .connection()
        .unwrap()
        .execute("vacuum into ?1", [database_path.to_str().unwrap()])
        .unwrap();
    let reopened = crate::storage::AppStore::for_test(
        rusqlite::Connection::open(database_path).unwrap(),
        f.store.content_directory().to_path_buf(),
    );
    let persisted = discovery::list(
        &reopened,
        discovery::ListRequest {
            manuscript_id: f.manuscript,
            after: None,
            limit: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(persisted).unwrap(), prepared);
    assert_eq!(prepared["items"][0]["undo"]["undoId"], json!(undo));
    assert_eq!(
        prepared["items"][0]["undo"]["originalScope"]["operationId"],
        json!(original.operation)
    );
    assert_eq!(prepared["items"][0]["undo"]["result"]["status"], "prepared");
    assert_eq!(prepared["nextCursor"], Value::Null);
    let cursor = prepared["items"][0]["id"].as_u64().unwrap();
    assert_eq!(page(&f, Some(cursor), 1).await["items"], json!([]));
    service
        .start_proposal_undo(original, undo, request())
        .await
        .unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"][0]["undo"]["result"]["status"],
        "applying"
    );
    service.defer_proposal_undo(original, undo).await.unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"][0]["undo"]["result"]["status"],
        "recoveryRequired"
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    service
        .cancel_pending_proposal_undo(original, undo)
        .await
        .unwrap();
    assert_eq!(page(&f, None, 64).await["items"], json!([]));
}

#[tokio::test]
async fn discovery_paginates_distinct_proposals_in_insertion_order() {
    let mut f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let first = existing(&f, false).await;
    service
        .apply_disk_proposal_file(first, "main.tex".into())
        .await
        .unwrap();
    let first_undo = Uuid::new_v4();
    service
        .prepare_proposal_undo(
            first,
            first_undo,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    f.hash = format!("{:x}", Sha256::digest(b"replacement"));
    let second = existing(&f, true).await;
    service
        .apply_disk_proposal_file(second, "main.tex".into())
        .await
        .unwrap();
    let second_undo = Uuid::new_v4();
    service
        .prepare_proposal_undo(
            second,
            second_undo,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let first_page = page(&f, None, 1).await;
    let cursor = first_page["nextCursor"].as_u64().unwrap();
    assert_eq!(first_page["items"][0]["undo"]["undoId"], json!(first_undo));
    let second_page = page(&f, Some(cursor), 1).await;
    assert_eq!(
        second_page["items"][0]["undo"]["undoId"],
        json!(second_undo)
    );
    assert!(second_page["items"][0]["id"].as_u64().unwrap() > cursor);
    assert_eq!(second_page["nextCursor"], Value::Null);
    assert!(!source(&f).join("main.tex").exists());
}

#[tokio::test]
async fn discovery_validates_manuscript_and_bounded_cursor_without_scope_leaks() {
    let f = fixture_with_objects();
    for (manuscript_id, after, limit) in [
        (Uuid::nil(), None, 1),
        (f.manuscript, None, 0),
        (f.manuscript, None, 65),
        (f.manuscript, Some(0), 1),
        (f.manuscript, Some(9_007_199_254_740_992), 1),
        (Uuid::new_v4(), None, 1),
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
    assert!(
        serde_json::from_value::<discovery::ListRequest>(
            json!({"manuscriptId":f.manuscript,"after":null,"limit":1,"extra":true})
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}

#[tokio::test]
async fn discovery_preserves_file_ownership_and_rejects_a_corrupt_inverse_plan() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let undo = Uuid::new_v4();
    service
        .prepare_proposal_undo(
            original,
            undo,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"][0]["undo"]["undoId"],
        json!(undo)
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=0 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"][0]["undo"]["undoId"],
        json!(undo)
    );
    f.store.connection().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)", params![f.conversation.to_string(), f.manuscript.to_string()]).unwrap();
    assert_eq!(
        page(&f, None, 64).await["items"].as_array().unwrap().len(),
        1
    );
    // Synthetic corruption bypasses immutability only in this disposable DB;
    // discovery must validate the entire inverse source plan before projecting it.
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger agent_proposal_undo_sources_immutable")
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_proposal_undo_sources set source_json='{}' where operation_id=?1",
            [undo.to_string()],
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
        b"replacement"
    );
}
