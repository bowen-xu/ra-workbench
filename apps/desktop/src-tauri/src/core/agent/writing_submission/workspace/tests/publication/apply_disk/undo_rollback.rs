use super::*;
use crate::agent::repository::change_sets;

#[tokio::test]
async fn rollback_partial_undo_restores_each_kind_without_reopening_original() {
    for kind in ["added", "removed", "replaced"] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = existing_with_extra(&f, kind == "added", true).await;
        for path in ["main.tex", "other.tex"] {
            service
                .apply_disk_proposal_file(original, path.into())
                .await
                .unwrap();
        }
        let operation = Uuid::new_v4();
        let request = || WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![],
        };
        service
            .prepare_proposal_undo(original, operation, request())
            .await
            .unwrap();
        service
            .start_proposal_undo(original, operation, request())
            .await
            .unwrap();
        let path = if kind == "removed" {
            "other.tex"
        } else {
            "main.tex"
        };
        service
            .apply_disk_undo_file(original, operation, path.into())
            .await
            .unwrap();
        let done = service
            .rollback_disk_undo_file(original, operation, path.into())
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::RolledBack);
        assert_eq!(
            done.application
                .files
                .iter()
                .find(|f| f.path == path)
                .unwrap()
                .phase,
            ApplicationPhase::Restored
        );
        match kind {
            "added" => assert!(!source(&f).join(path).exists()),
            "removed" => assert_eq!(std::fs::read(source(&f).join(path)).unwrap(), b"extra"),
            _ => assert_eq!(
                std::fs::read(source(&f).join(path)).unwrap(),
                b"replacement"
            ),
        }
        assert_eq!(
            service
                .read_proposal_application(
                    original.conversation,
                    original.manuscript,
                    original.proposal,
                    original.operation
                )
                .await
                .unwrap()
                .status,
            ApplicationStatus::Applied
        );
        assert_eq!(
            change_sets::read_in(
                &f.store.connection().unwrap(),
                original.conversation,
                original.manuscript,
                original.proposal
            )
            .unwrap()
            .status,
            change_sets::ChangeSetStatus::Applied
        );
        std::fs::write(source(&f).join(path), b"later user input").unwrap();
        assert_eq!(
            service
                .rollback_disk_undo_file(original, operation, path.into())
                .await
                .unwrap()
                .application,
            done.application
        );
        assert_eq!(
            std::fs::read(source(&f).join(path)).unwrap(),
            b"later user input"
        );
    }
}

#[tokio::test]
async fn undo_rollback_recovers_lost_confirmation_and_refuses_later_edits() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing_with_extra(&f, false, true).await;
    for path in ["main.tex", "other.tex"] {
        service
            .apply_disk_proposal_file(original, path.into())
            .await
            .unwrap();
    }
    let operation = Uuid::new_v4();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    service
        .prepare_proposal_undo(original, operation, request())
        .await
        .unwrap();
    service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    service
        .apply_disk_undo_file(original, operation, "main.tex".into())
        .await
        .unwrap();
    f.store.connection().unwrap().execute_batch("create trigger fail_undo_restore before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .rollback_disk_undo_file(original, operation, "main.tex".into())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_undo_restore;")
        .unwrap();
    std::fs::write(source(&f).join("main.tex"), b"later input").unwrap();
    assert!(
        service
            .rollback_disk_undo_file(original, operation, "main.tex".into())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"later input"
    );
    let state = service
        .read_proposal_undo(original, operation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert_eq!(
        state.application.files[0].phase,
        ApplicationPhase::Acknowledged
    );
    std::fs::write(source(&f).join("main.tex"), b"replacement").unwrap();
    assert_eq!(
        service
            .rollback_disk_undo_file(original, operation, "main.tex".into())
            .await
            .unwrap()
            .application
            .status,
        ApplicationStatus::RolledBack
    );
}
