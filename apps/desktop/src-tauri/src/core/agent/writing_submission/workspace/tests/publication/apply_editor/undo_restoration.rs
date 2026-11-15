use super::*;
use crate::agent::repository::proposal_applications::ApplicationPhase;
use crate::agent::writing_submission::workspace::UndoEditorAdmission;

#[tokio::test]
async fn undo_editor_compensation_recovers_receipts_and_preserves_original() {
    exercise(false, false, ChatState::Attached).await;
}

#[tokio::test]
async fn completed_undo_cannot_enter_editor_compensation() {
    exercise(true, false, ChatState::Attached).await;
}

#[tokio::test]
async fn mixed_undo_rollback_rechecks_every_target_before_editor_and_disk() {
    exercise(false, true, ChatState::Attached).await;
}

#[tokio::test]
async fn detached_chat_keeps_mixed_undo_compensation_and_conflict_protection() {
    exercise(false, true, ChatState::Detached).await;
}

async fn exercise(complete: bool, mixed: bool, chat: ChatState) {
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
    if mixed {
        std::fs::write(
            copy.lease.working_directory().unwrap().join("a.tex"),
            b"disk original",
        )
        .unwrap();
    }
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
    if mixed {
        service
            .apply_disk_proposal_file(scope, "a.tex".into())
            .await
            .unwrap();
    }
    let undo_id = Uuid::new_v4();
    if matches!(chat, ChatState::Detached) {
        let db = f.store.connection().unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
        db.execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    }
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
    assert!(
        service
            .prepare_undo_editor_restoration(
                scope,
                undo_id,
                Uuid::new_v4(),
                observe(restored.clone(), &dispatch.text)
            )
            .await
            .is_err()
    );
    let confirmed = service
        .confirm_editor_undo(scope, undo_id, observe(restored.clone(), &dispatch.text))
        .await
        .unwrap();
    assert_eq!(confirmed.application.status, ApplicationStatus::Applying);
    if mixed {
        service
            .apply_disk_undo_file(scope, undo_id, "a.tex".into())
            .await
            .unwrap();
    }
    use crate::agent::writing_submission::workspace::EditorRestoreAdmission;
    let restore_id = Uuid::new_v4();
    if complete {
        service
            .apply_disk_undo_file(scope, undo_id, "z.tex".into())
            .await
            .unwrap();
        assert!(
            service
                .prepare_undo_editor_restoration(
                    scope,
                    undo_id,
                    restore_id,
                    observe(restored, "unsaved before")
                )
                .await
                .is_err()
        );
        assert!(
            service
                .read_undo_editor_restoration(scope, undo_id, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
        return;
    }
    for observation in [
        observe(restored.clone(), "later input"),
        observe(applied, "unsaved before"),
        observe(
            EditorVersion {
                session_id: Uuid::new_v4(),
                revision: restored.revision,
            },
            "unsaved before",
        ),
    ] {
        assert!(
            service
                .prepare_undo_editor_restoration(scope, undo_id, restore_id, observation)
                .await
                .is_err()
        );
        assert!(
            service
                .read_undo_editor_restoration(scope, undo_id, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
    }
    assert!(
        service
            .read_undo_editor_restoration(
                ApplicationScope {
                    conversation: Uuid::new_v4(),
                    ..scope
                },
                undo_id,
                "main.tex".into()
            )
            .await
            .is_err()
    );
    for invalid_id in [Uuid::nil(), scope.operation, undo_id] {
        assert!(
            service
                .prepare_undo_editor_restoration(
                    scope,
                    undo_id,
                    invalid_id,
                    observe(restored.clone(), "unsaved before")
                )
                .await
                .is_err()
        );
    }
    use crate::agent::writing_application_commands::undo_restoration as boundary;
    let original_scope = serde_json::json!({"conversationId":scope.conversation,"manuscriptId":scope.manuscript,"proposalId":scope.proposal,"operationId":scope.operation});
    let mut undo_scope = original_scope.clone();
    undo_scope["operationId"] = undo_id.to_string().into();
    let wire = serde_json::json!({"originalScope":original_scope,"request":{"action":"prepare","scope":undo_scope,"restoreId":restore_id,"current":{"path":"main.tex","sessionId":restored.session_id,"revision":restored.revision,"text":"unsaved before"}}});
    let mut same_operation = wire.clone();
    same_operation["request"]["scope"] = original_scope.clone();
    let mut wrong_manuscript = wire.clone();
    wrong_manuscript["request"]["scope"]["manuscriptId"] = Uuid::new_v4().to_string().into();
    for invalid in [same_operation, wrong_manuscript] {
        assert!(
            boundary::execute(&f.store, serde_json::from_value(invalid).unwrap())
                .await
                .is_err()
        );
    }
    if mixed {
        std::fs::write(source.join("z.tex"), b"user changed pending").unwrap();
        assert!(
            service
                .prepare_proposal_undo_rollback(
                    scope,
                    undo_id,
                    buffers(restored.clone(), "unsaved before")
                )
                .await
                .is_err()
        );
        assert!(
            service
                .read_undo_editor_restoration(scope, undo_id, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
        assert!(!source.join("a.tex").exists());
        std::fs::write(source.join("z.tex"), b"new file").unwrap();
        let gate = service
            .prepare_proposal_undo_rollback(
                scope,
                undo_id,
                buffers(restored.clone(), "unsaved before"),
            )
            .await
            .unwrap();
        assert_eq!(gate.application.status, ApplicationStatus::RecoveryRequired);
        assert!(
            service
                .rollback_disk_proposal_undo(
                    scope,
                    undo_id,
                    buffers(restored.clone(), "unsaved before")
                )
                .await
                .is_err()
        );
        assert!(!source.join("a.tex").exists());
    }
    let edit = serde_json::to_value(
        boundary::execute(&f.store, serde_json::from_value(wire).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(edit["originalScope"], original_scope);
    assert_eq!(edit["result"]["scope"], undo_scope);
    assert_eq!(edit["result"]["kind"], "edit");
    assert_eq!(edit["result"]["restoreId"], restore_id.to_string());
    assert_eq!(edit["result"]["text"], "proposal");
    let (saved, restoration) = service
        .read_undo_editor_restoration(scope, undo_id, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        saved.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert!(
        service
            .resume_proposal_undo(scope, undo_id, buffers(restored.clone(), "unsaved before"))
            .await
            .is_err()
    );
    assert_eq!(restoration.unwrap().expected, restored);
    let result = EditorVersion {
        session_id: restored.session_id,
        revision: Uuid::new_v4(),
    };
    for observation in [
        observe(restored.clone(), "proposal"),
        observe(result.clone(), "wrong text"),
        observe(
            EditorVersion {
                session_id: Uuid::new_v4(),
                ..result.clone()
            },
            "proposal",
        ),
    ] {
        assert!(
            service
                .confirm_undo_editor_restoration(scope, undo_id, restore_id, observation)
                .await
                .is_err()
        );
    }
    f.store.connection().unwrap().execute_batch("create trigger fail_undo_restore_ack before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .confirm_undo_editor_restoration(
                scope,
                undo_id,
                restore_id,
                observe(result.clone(), "proposal")
            )
            .await
            .is_err()
    );
    let (saved, restoration) = service
        .read_undo_editor_restoration(scope, undo_id, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        saved.application.files[0].phase,
        ApplicationPhase::Acknowledged
    );
    assert_eq!(restoration.unwrap().result_revision, None);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_undo_restore_ack;")
        .unwrap();
    let service = WritingSubmissionService::from_app_store(&f.store);
    assert!(matches!(
        service
            .prepare_undo_editor_restoration(
                scope,
                undo_id,
                restore_id,
                observe(result.clone(), "later input")
            )
            .await
            .unwrap(),
        EditorRestoreAdmission::Existing { .. }
    ));
    assert!(
        service
            .prepare_undo_editor_restoration(
                scope,
                undo_id,
                Uuid::new_v4(),
                observe(result.clone(), "proposal")
            )
            .await
            .is_err()
    );
    let confirmation = serde_json::json!({"originalScope":original_scope,"request":{"action":"confirm","scope":undo_scope,"restoreId":restore_id,"actual":{"path":"main.tex","sessionId":result.session_id,"revision":result.revision,"text":"proposal"}}});
    let confirmed = serde_json::to_value(
        boundary::execute(&f.store, serde_json::from_value(confirmation).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        confirmed["result"]["restoration"]["resultRevision"],
        result.revision.to_string()
    );
    if mixed {
        for current in [
            buffers(restored.clone(), "proposal"),
            buffers(result.clone(), "later input"),
        ] {
            assert!(
                service
                    .rollback_disk_proposal_undo(scope, undo_id, current)
                    .await
                    .is_err()
            );
            assert!(!source.join("a.tex").exists());
        }
        std::fs::write(source.join("z.tex"), b"later pending edit").unwrap();
        assert!(
            service
                .rollback_disk_proposal_undo(scope, undo_id, buffers(result.clone(), "proposal"))
                .await
                .is_err()
        );
        assert!(!source.join("a.tex").exists());
        std::fs::write(source.join("z.tex"), b"new file").unwrap();
        service
            .rollback_disk_proposal_undo(scope, undo_id, buffers(result.clone(), "proposal"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(source.join("a.tex")).unwrap(),
            b"disk original"
        );
    }
    let done = service
        .read_proposal_undo(scope, undo_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::RolledBack);
    assert_eq!(
        done.application
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .phase,
        ApplicationPhase::Restored
    );
    assert_eq!(
        done.application
            .files
            .iter()
            .find(|f| f.path == "z.tex")
            .unwrap()
            .phase,
        ApplicationPhase::Pending
    );
    assert_eq!(
        service
            .confirm_undo_editor_restoration(
                scope,
                undo_id,
                restore_id,
                observe(result, "proposal")
            )
            .await
            .unwrap()
            .application,
        done.application
    );
    let read = serde_json::json!({"originalScope":original_scope,"request":{"action":"read","scope":undo_scope,"path":"main.tex"}});
    let historical = serde_json::to_value(
        boundary::execute(&f.store, serde_json::from_value(read).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(historical["result"]["application"]["scope"], undo_scope);
    assert_eq!(historical["result"]["application"]["status"], "rolledBack");
    assert_eq!(
        historical["result"]["restoration"]["id"],
        restore_id.to_string()
    );
    let db = f.store.connection().unwrap();
    let original = crate::agent::repository::proposal_applications::read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(original.status, ApplicationStatus::Applied);
    assert!(
        original
            .files
            .iter()
            .all(|f| f.phase == ApplicationPhase::Acknowledged)
    );
    assert_eq!(
        crate::agent::repository::change_sets::read_file_operation_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal
        )
        .unwrap()
        .status,
        crate::agent::repository::change_sets::ChangeSetStatus::Applied
    );
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert_eq!(std::fs::read(source.join("z.tex")).unwrap(), b"new file");
}
