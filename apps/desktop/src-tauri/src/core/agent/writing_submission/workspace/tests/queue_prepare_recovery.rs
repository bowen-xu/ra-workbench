//! A prepared queue record may legitimately precede its execution workspace.
use super::*;
use crate::agent::{
    writing_queue_preview::{self, QueuePreviewRequest},
    writing_submission::tests::{managed_fixture, queue_preview_intent},
    writing_workspace_commands as commands,
};
use research_agent_core::input_queue::{QueueEdit, QueuedInput};

#[tokio::test]
async fn original_queued_prepare_without_workspace_returns_not_found_then_prepares_and_restores_exact_copy()
 {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let input_id = Uuid::new_v4();
    let input = request(&f, Uuid::new_v4());
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    let intent = queue_preview_intent(&input, input_id, 1);
    let wire: QueuePreviewRequest = serde_json::from_value(intent.request_json).unwrap();
    let prepared = writing_queue_preview::prepare(&service, &f.models, wire)
        .await
        .unwrap();
    let scope = commands::Scope {
        conversation_id: f.conversation,
        request_id: prepared.preview.request_id,
        manuscript_id: f.manuscript,
        context_snapshot_id: prepared.preview.context_snapshot_id,
    };
    assert!(matches!(
        service
            .restore_workspace(scope.receipt(), scope.context_snapshot_id)
            .await,
        Err(Error::NotFound)
    ));
    let missing = commands::restore(&f.store, scope.clone())
        .await
        .err()
        .unwrap();
    assert!(matches!(missing, commands::Error::NotFound));
    assert_eq!(serde_json::to_value(missing).unwrap(), "notFound");
    assert!(!f.store.agent_workspace_directory().exists());

    let created = serde_json::to_value(
        commands::prepare(
            &f.store,
            commands::PrepareRequest {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    let restored =
        serde_json::to_value(commands::restore(&f.store, scope.clone()).await.unwrap()).unwrap();
    assert_eq!(created, restored);
    assert_eq!(restored["requestId"], intent.request_id.to_string());
    assert_eq!(
        restored["contextSnapshotId"],
        scope.context_snapshot_id.to_string()
    );
    assert_eq!(restored["approved"], true);
    assert!(!restored.to_string().contains("Frozen manuscript"));

    let mut wrong_snapshot = scope.clone();
    wrong_snapshot.context_snapshot_id = Uuid::new_v4();
    assert!(matches!(
        commands::restore(&f.store, wrong_snapshot).await,
        Err(commands::Error::Conflict)
    ));
    assert_eq!(
        serde_json::to_value(commands::restore(&f.store, scope.clone()).await.unwrap()).unwrap(),
        created
    );

    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    let denied = commands::restore(&f.store, scope).await.err().unwrap();
    assert!(matches!(denied, commands::Error::Denied));
    assert_eq!(serde_json::to_value(denied).unwrap(), "denied");
}
