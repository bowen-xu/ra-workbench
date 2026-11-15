//! Frozen history is not the lifetime of the editor receiving identical bytes.
use super::*;
use crate::agent::repository::proposal_applications::{ApplicationScope, ApplicationTarget};
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;

fn editor_version() -> EditorVersion {
    EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    }
}

async fn frozen_editor_change(
    f: &crate::agent::writing_submission::tests::Fixture,
) -> WorkspaceReview {
    let service = WritingSubmissionService::from_app_store(&f.store);
    let editor = editor_version();
    let mut input = request(f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: editor.revision,
        text: "Frozen manuscript".into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            prepared.record.context_snapshot_id,
            buffers(f.manuscript, editor, "Frozen manuscript"),
        )
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("main.tex"),
        "Revised manuscript",
    )
    .unwrap();
    drop(copy);
    service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap()
}

fn buffers(manuscript: Uuid, editor: EditorVersion, text: &str) -> WritingWorkspaceRequest {
    WritingWorkspaceRequest {
        manuscript_id: manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
            editor,
            text: text.into(),
        }],
    }
}

#[tokio::test]
async fn file_opened_after_freezing_receives_the_change_in_its_current_editor() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    assert!(review.changes[0].source.as_ref().unwrap().editor.is_none());
    let original = review.changes.clone();
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let editor = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
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
            buffers(f.manuscript, editor.clone(), "Frozen manuscript"),
        )
        .await
        .unwrap();
    let saved = service
        .read_proposal_application(f.conversation, f.manuscript, proposal, scope.operation)
        .await
        .unwrap();
    assert_eq!(
        saved.files[0].target,
        ApplicationTarget::Editor {
            expected: editor.clone(),
            result_revision: None
        }
    );
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, proposal)
            .await
            .unwrap()
            .changes,
        original
    );
    assert!(
        service
            .apply_disk_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    // Admission seals the live CAS; it is not permission to overwrite input
    // that arrived while the application was waiting to dispatch.
    assert!(
        service
            .prepare_editor_proposal_file(
                scope,
                EditorObservation {
                    path: "main.tex".into(),
                    version: EditorVersion {
                        session_id: editor.session_id,
                        revision: Uuid::new_v4()
                    },
                    text: "later user edit".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        service
            .begin_proposal_application(
                f.conversation,
                proposal,
                scope.operation,
                buffers(f.manuscript, editor.clone(), "later user edit")
            )
            .await
            .unwrap(),
        crate::agent::repository::proposal_applications::ApplicationAdmission::Existing(
            scope.operation
        )
    );
    let admission = service
        .prepare_editor_proposal_file(
            scope,
            EditorObservation {
                path: "main.tex".into(),
                version: editor,
                text: "Frozen manuscript".into(),
            },
        )
        .await
        .unwrap();
    let EditorAdmission::Dispatch(edit) = admission else {
        panic!("expected current editor dispatch")
    };
    assert_eq!(edit.text, "Revised manuscript");
}

#[tokio::test]
async fn restored_editor_can_receive_unchanged_frozen_bytes_without_rewriting_history() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = frozen_editor_change(&f).await;
    let historical = review.changes.clone();
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let editor = editor_version();
    assert_ne!(
        historical[0].source.as_ref().unwrap().editor.as_ref(),
        Some(&editor)
    );
    assert!(
        service
            .begin_proposal_application(
                f.conversation,
                proposal,
                Uuid::new_v4(),
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
    let operation = Uuid::new_v4();
    service
        .begin_proposal_application(
            f.conversation,
            proposal,
            operation,
            buffers(f.manuscript, editor.clone(), "Frozen manuscript"),
        )
        .await
        .unwrap();
    let saved = service
        .read_proposal_application(f.conversation, f.manuscript, proposal, operation)
        .await
        .unwrap();
    assert_eq!(
        saved.files[0].target,
        ApplicationTarget::Editor {
            expected: editor,
            result_revision: None
        }
    );
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, proposal)
            .await
            .unwrap()
            .changes,
        historical
    );
}

#[tokio::test]
async fn recovery_renews_only_pending_targets_after_full_text_and_disk_preflight() {
    use crate::agent::repository::proposal_applications::{self, ApplicationStatus};
    for from_editor in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let review = if from_editor {
            frozen_editor_change(&f).await
        } else {
            changed(&f).await
        };
        let historical = review.changes.clone();
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
        let request = if from_editor {
            buffers(
                f.manuscript,
                historical[0]
                    .source
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone()
                    .unwrap(),
                "Frozen manuscript",
            )
        } else {
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            }
        };
        service
            .begin_proposal_application(f.conversation, proposal, scope.operation, request)
            .await
            .unwrap();
        let previous = service
            .read_proposal_application(f.conversation, f.manuscript, proposal, scope.operation)
            .await
            .unwrap();
        let editor = editor_version();
        // An active owner's CAS cannot be replaced by another editor.
        assert!(
            service
                .resume_proposal_application(
                    scope,
                    buffers(f.manuscript, editor.clone(), "Frozen manuscript")
                )
                .await
                .is_err()
        );
        assert_eq!(
            proposal_applications::recover_on_startup(&mut f.store.connection().unwrap()).unwrap(),
            1
        );
        assert!(
            service
                .resume_proposal_application(
                    scope,
                    buffers(f.manuscript, editor.clone(), "later user edit")
                )
                .await
                .is_err()
        );
        assert_eq!(
            service
                .read_proposal_application(f.conversation, f.manuscript, proposal, scope.operation)
                .await
                .unwrap()
                .files,
            previous.files
        );
        let resumed = service
            .resume_proposal_application(
                scope,
                buffers(f.manuscript, editor.clone(), "Frozen manuscript"),
            )
            .await
            .unwrap();
        assert_eq!(resumed.operation_id, scope.operation);
        assert_eq!(resumed.status, ApplicationStatus::Applying);
        assert_eq!(
            resumed.files[0].target,
            ApplicationTarget::Editor {
                expected: editor.clone(),
                result_revision: None
            }
        );
        assert_eq!(
            service
                .review_proposal(f.conversation, f.manuscript, proposal)
                .await
                .unwrap()
                .changes,
            historical
        );
        let admitted = service
            .prepare_editor_proposal_file(
                scope,
                EditorObservation {
                    path: "main.tex".into(),
                    version: editor.clone(),
                    text: "Frozen manuscript".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(admitted, EditorAdmission::Dispatch(_)));
        proposal_applications::recover_on_startup(&mut f.store.connection().unwrap()).unwrap();
        // A dispatched operation must reconcile its original receipt.
        assert!(
            service
                .resume_proposal_application(
                    scope,
                    buffers(f.manuscript, editor_version(), "Frozen manuscript")
                )
                .await
                .is_err()
        );
        assert_eq!(
            service
                .read_proposal_application(f.conversation, f.manuscript, proposal, scope.operation)
                .await
                .unwrap()
                .files[0]
                .target,
            ApplicationTarget::Editor {
                expected: editor,
                result_revision: None
            }
        );
    }
}

#[tokio::test]
async fn current_editor_binding_does_not_bypass_frozen_text_or_disk_guards() {
    for text in ["later user edit", "Frozen manuscript"] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let review = changed(&f).await;
        let proposal = service
            .publish_workspace_changes(review.binding, review.changes)
            .await
            .unwrap();
        let editor = EditorVersion {
            session_id: Uuid::new_v4(),
            revision: Uuid::new_v4(),
        };
        if text == "Frozen manuscript" {
            std::fs::write(
                f.store
                    .manuscript_directory()
                    .join(f.manuscript.to_string())
                    .join("source/main.tex"),
                "external edit",
            )
            .unwrap();
        }
        assert!(
            service
                .begin_proposal_application(
                    f.conversation,
                    proposal,
                    Uuid::new_v4(),
                    buffers(f.manuscript, editor, text)
                )
                .await
                .is_err()
        );
        assert!(
            service
                .find_proposal_application(f.conversation, f.manuscript, proposal)
                .await
                .unwrap()
                .is_none()
        );
    }
}
