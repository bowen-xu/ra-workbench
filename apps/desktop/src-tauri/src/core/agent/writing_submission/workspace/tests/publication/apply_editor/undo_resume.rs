use super::*;
use crate::agent::repository::proposal_applications::ApplicationPhase;
use crate::agent::writing_submission::workspace::UndoEditorAdmission;

#[tokio::test]
async fn resume_keeps_confirmed_editor_receipt_and_revalidates_current_buffer() {
    let f = fixture_with_objects();
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    let service = WritingSubmissionService::from_app_store(&f.store);
    let editor = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: editor.revision,
        text: "unsaved before".into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let buffers = |version: EditorVersion, text: &str| WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: f.hash.clone(),
            editor: version,
            text: text.into(),
        }],
    };
    let observe = |version: EditorVersion, text: &str| EditorObservation {
        path: "main.tex".into(),
        version,
        text: text.into(),
    };
    let copy = service
        .prepare_workspace(
            receipt,
            prepared.record.context_snapshot_id,
            buffers(editor.clone(), "unsaved before"),
        )
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("main.tex"),
        b"proposal",
    )
    .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("z.tex"),
        b"new file",
    )
    .unwrap();
    drop(copy);
    let review = service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap();
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let scope = ApplicationScope {
        conversation: f.conversation,
        manuscript: f.manuscript,
        proposal,
        operation: Uuid::new_v4(),
    };
    service
        .begin_proposal_application(
            f.conversation,
            proposal,
            scope.operation,
            buffers(editor.clone(), "unsaved before"),
        )
        .await
        .unwrap();
    service
        .prepare_editor_proposal_file(scope, observe(editor.clone(), "unsaved before"))
        .await
        .unwrap();
    let applied = EditorVersion {
        session_id: editor.session_id,
        revision: Uuid::new_v4(),
    };
    service
        .confirm_editor_proposal_file(scope, observe(applied.clone(), "proposal"))
        .await
        .unwrap();
    service
        .apply_disk_proposal_file(scope, "z.tex".into())
        .await
        .unwrap();
    let undo_id = Uuid::new_v4();
    service
        .prepare_proposal_undo(scope, undo_id, buffers(applied.clone(), "proposal"))
        .await
        .unwrap();
    service
        .start_proposal_undo(scope, undo_id, buffers(applied.clone(), "proposal"))
        .await
        .unwrap();
    let UndoEditorAdmission::Dispatch(dispatch) = service
        .prepare_editor_undo(scope, undo_id, observe(applied.clone(), "proposal"))
        .await
        .unwrap()
    else {
        panic!("expected editor dispatch")
    };
    assert_eq!(dispatch.text, "unsaved before");
    let restored = EditorVersion {
        session_id: editor.session_id,
        revision: Uuid::new_v4(),
    };
    let confirmed = service
        .confirm_editor_undo(scope, undo_id, observe(restored.clone(), &dispatch.text))
        .await
        .unwrap();
    assert_eq!(confirmed.application.status, ApplicationStatus::Applying);
    service.defer_proposal_undo(scope, undo_id).await.unwrap();
    let missing = WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    assert!(
        service
            .resume_proposal_undo(scope, undo_id, missing)
            .await
            .is_err()
    );
    for request in [
        buffers(restored.clone(), "later input"),
        buffers(applied, "unsaved before"),
        buffers(
            EditorVersion {
                session_id: Uuid::new_v4(),
                revision: restored.revision,
            },
            "unsaved before",
        ),
    ] {
        assert!(
            service
                .resume_proposal_undo(scope, undo_id, request)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(source.join("z.tex")).unwrap(), b"new file");
    }
    let resumed = service
        .resume_proposal_undo(scope, undo_id, buffers(restored, "unsaved before"))
        .await
        .unwrap();
    assert_eq!(resumed.application.files, confirmed.application.files);
    assert_eq!(
        resumed.application.files[0].phase,
        ApplicationPhase::Acknowledged
    );
    let done = service
        .apply_disk_undo_file(scope, undo_id, "z.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::Applied);
    assert!(!source.join("z.tex").exists());
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}
