use super::*;

#[tokio::test]
async fn undo_preview_requires_completed_effect_and_keeps_later_user_edits() {
    for removed in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let scope = existing(&f, removed).await;
        let request = || WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![],
        };
        assert!(
            service
                .preview_proposal_undo(scope, request())
                .await
                .is_err()
        );
        service
            .apply_disk_proposal_file(scope, "main.tex".into())
            .await
            .unwrap();
        let inverse = service
            .preview_proposal_undo(scope, request())
            .await
            .unwrap();
        assert_eq!(inverse.len(), 1);
        assert_eq!(inverse[0].path, "main.tex");
        assert_eq!(inverse[0].before.is_none(), removed);
        assert_eq!(inverse[0].source.is_none(), removed);
        assert_eq!(
            inverse[0].after.as_ref().unwrap().sha256,
            format!("{:x}", Sha256::digest(b"Frozen manuscript"))
        );
        let saved = service
            .read_proposal_application(
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )
            .await
            .unwrap();
        assert_eq!(saved.status, ApplicationStatus::Applied);
        assert!(
            service
                .preview_proposal_undo(
                    scope,
                    WritingWorkspaceRequest {
                        manuscript_id: Uuid::new_v4(),
                        buffers: vec![]
                    }
                )
                .await
                .is_err()
        );
        std::fs::write(source(&f).join("main.tex"), b"later user edit").unwrap();
        assert!(
            service
                .preview_proposal_undo(scope, request())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later user edit"
        );
    }
}
