use super::*;
use crate::agent::repository::proposal_applications::ApplicationPhase;
use crate::agent::writing_submission::workspace::UndoEditorAdmission;

pub(super) async fn exercise(
    f: &crate::agent::writing_submission::tests::Fixture,
    service: &WritingSubmissionService,
    original: ApplicationScope,
    operation: Uuid,
    version: EditorVersion,
    text: &str,
) {
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: f.hash.clone(),
            editor: version.clone(),
            text: text.into(),
        }],
    };
    let observed = |version: EditorVersion, text: &str| EditorObservation {
        path: "main.tex".into(),
        version,
        text: text.into(),
    };
    assert!(
        service
            .prepare_editor_undo(original, operation, observed(version.clone(), text))
            .await
            .is_err()
    );
    let mut stale = request();
    stale.buffers[0].editor.revision = Uuid::new_v4();
    assert!(
        service
            .start_proposal_undo(original, operation, stale)
            .await
            .is_err()
    );
    let started = service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    assert_eq!(started.application.status, ApplicationStatus::Applying);
    service
        .defer_proposal_undo(original, operation)
        .await
        .unwrap();
    let mut stale = request();
    stale.buffers[0].editor.revision = Uuid::new_v4();
    assert!(
        service
            .resume_proposal_undo(original, operation, stale)
            .await
            .is_err()
    );
    let mut missing = request();
    missing.buffers.clear();
    assert!(
        service
            .resume_proposal_undo(original, operation, missing)
            .await
            .is_err()
    );
    assert_eq!(
        service
            .resume_proposal_undo(original, operation, request())
            .await
            .unwrap()
            .application
            .status,
        ApplicationStatus::Applying
    );
    assert!(
        service
            .prepare_editor_undo(
                original,
                operation,
                observed(version.clone(), "later input")
            )
            .await
            .is_err()
    );
    let UndoEditorAdmission::Dispatch(edit) = service
        .prepare_editor_undo(original, operation, observed(version.clone(), text))
        .await
        .unwrap()
    else {
        panic!("new undo edit");
    };
    assert_eq!(edit.operation, operation);
    assert_eq!(edit.expected.version, version);
    assert_eq!(edit.text, "unsaved baseline");
    assert!(matches!(
        service
            .prepare_editor_undo(
                original,
                operation,
                observed(version.clone(), "later input")
            )
            .await
            .unwrap(),
        UndoEditorAdmission::Existing(_)
    ));
    // A new process cannot resume the departed process's dispatch automatically.
    let _sessions = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let stopped = service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    assert_eq!(
        stopped.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert_eq!(
        stopped.application.files[0].phase,
        ApplicationPhase::Dispatched
    );
    let result = EditorVersion {
        session_id: version.session_id,
        revision: Uuid::new_v4(),
    };
    assert!(
        service
            .confirm_editor_undo(original, operation, observed(version.clone(), &edit.text))
            .await
            .is_err()
    );
    assert!(
        service
            .confirm_editor_undo(original, operation, observed(result.clone(), "wrong bytes"))
            .await
            .is_err()
    );
    assert!(
        service
            .confirm_editor_undo(
                original,
                operation,
                observed(
                    EditorVersion {
                        session_id: Uuid::new_v4(),
                        revision: result.revision
                    },
                    &edit.text
                )
            )
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute_batch("create trigger deny_undo_finish before update of status on agent_change_sets when new.status='undone' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .confirm_editor_undo(original, operation, observed(result.clone(), &edit.text))
            .await
            .is_err()
    );
    let saved = service
        .read_proposal_undo(original, operation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert_eq!(
        saved.application.files[0].phase,
        ApplicationPhase::Dispatched
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger deny_undo_finish")
        .unwrap();
    let finished = service
        .confirm_editor_undo(original, operation, observed(result.clone(), &edit.text))
        .await
        .unwrap();
    assert_eq!(finished.application.status, ApplicationStatus::Applied);
    assert_eq!(
        service
            .confirm_editor_undo(original, operation, observed(result, &edit.text))
            .await
            .unwrap()
            .application,
        finished.application
    );
    assert!(matches!(
        service
            .prepare_editor_undo(
                original,
                operation,
                observed(version.clone(), "later input")
            )
            .await
            .unwrap(),
        UndoEditorAdmission::Existing(_)
    ));
    // The original receipt stays historical, including after successful undo.
    assert_eq!(
        service
            .confirm_editor_proposal_file(original, observed(version, text))
            .await
            .unwrap()
            .status,
        ApplicationStatus::Applied
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from agent_change_sets where id=?1",
                [original.proposal.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "undone"
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
}
