use super::*;

#[tokio::test]
async fn undo_preview_of_added_binary_retains_exact_removal_baseline() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let bytes = [0xff, 0, 0xfe];
    let scope = added(&f, &bytes).await;
    service
        .apply_added_proposal_file(scope, "new.bin".into())
        .await
        .unwrap();
    let inverse = service
        .preview_proposal_undo(
            scope,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(inverse.len(), 1);
    assert_eq!(inverse[0].path, "new.bin");
    assert_eq!(
        inverse[0].kind,
        research_agent_core::workspace::FileChangeKind::Deleted
    );
    assert!(inverse[0].after.is_none());
    assert_eq!(
        inverse[0].before.as_ref().unwrap().sha256,
        format!("{:x}", Sha256::digest(bytes))
    );
    assert!(inverse[0].source.is_some());
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/new.bin")
        )
        .unwrap(),
        bytes
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert_eq!(
        service
            .preview_proposal_undo(
                scope,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .unwrap(),
        inverse
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/new.bin")
        )
        .unwrap(),
        bytes
    );
}
