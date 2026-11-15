use super::*;

#[tokio::test]
async fn whole_undo_rollback_preflights_untouched_files_and_open_buffers() {
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
    std::fs::write(source(&f).join("other.tex"), b"user changed pending file").unwrap();
    assert!(
        service
            .rollback_disk_proposal_undo(original, operation, request())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    std::fs::write(source(&f).join("other.tex"), b"extra").unwrap();
    let mut open = request();
    open.buffers
        .push(crate::agent::writing_workspace::EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: f.hash.clone(),
            text: "Frozen manuscript".into(),
            editor: research_agent_core::workspace::EditorVersion {
                session_id: Uuid::new_v4(),
                revision: Uuid::new_v4(),
            },
        });
    assert!(
        service
            .rollback_disk_proposal_undo(original, operation, open)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    let command = serde_json::json!({"scope":{"conversationId":original.conversation,"manuscriptId":original.manuscript,"proposalId":original.proposal,"operationId":original.operation},"undoId":operation,"command":{"action":"rollbackDisk","buffers":[]}});
    let response = crate::agent::writing_application_commands::undo::execute(
        &f.store,
        serde_json::from_value(command).unwrap(),
    )
    .await
    .unwrap();
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["result"]["status"], "rolledBack");
    assert_eq!(
        response["result"]["scope"]["operationId"],
        operation.to_string()
    );
    assert_eq!(
        response["originalScope"]["operationId"],
        original.operation.to_string()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn whole_undo_rollback_resumes_after_partial_compensation() {
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
    f.store.connection().unwrap().execute_batch("create trigger fail_finish_undo before update of status on agent_proposal_applications when new.direction='undo' and new.status='applied' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_disk_undo_file(original, operation, "other.tex".into())
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute_batch("drop trigger fail_finish_undo; create trigger fail_restore_main before update of phase on agent_proposal_application_files when new.relative_path='main.tex' and new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .rollback_disk_proposal_undo(original, operation, request())
            .await
            .is_err()
    );
    let partial = service
        .read_proposal_undo(original, operation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        partial.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert_eq!(
        partial.application.files[1].phase,
        ApplicationPhase::Restored
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_restore_main;")
        .unwrap();
    let done = service
        .rollback_disk_proposal_undo(original, operation, request())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::RolledBack);
    assert!(
        done.application
            .files
            .iter()
            .all(|f| f.phase == ApplicationPhase::Restored)
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        std::fs::read(source(&f).join("other.tex")).unwrap(),
        b"extra"
    );
    std::fs::write(source(&f).join("main.tex"), b"later").unwrap();
    assert_eq!(
        service
            .rollback_disk_proposal_undo(original, operation, request())
            .await
            .unwrap()
            .application,
        done.application
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"later"
    );
}
