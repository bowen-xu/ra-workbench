use super::*;
use crate::agent::repository::proposal_applications::{
    self as applications, ApplicationPhase, ApplicationScope, ApplicationStatus,
};
use crate::agent::writing_application_commands::restoration as boundary;
use crate::agent::writing_submission::workspace::{
    EditorAdmission, EditorObservation, EditorRestoreAdmission,
};
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;

#[path = "restore_editor/checkpoint.rs"]
mod checkpoint;

#[tokio::test]
async fn restoration_validates_receipts_recovers_failed_ack_and_preserves_disk() {
    exercise_restoration(false).await;
}

#[tokio::test]
async fn mixed_editor_disk_rollback_waits_for_editor_receipt_and_preserves_conflicts() {
    exercise_restoration(true).await;
}

async fn exercise_restoration(mixed: bool) {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let version = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let before = "未保存 baseline\r\n";
    let after = "proposal 中文\r\n";
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: version.revision,
        text: before.into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let buffers = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
            editor: version.clone(),
            text: before.into(),
        }],
    };
    let copy = service
        .prepare_workspace(receipt, prepared.record.context_snapshot_id, buffers())
        .await
        .unwrap();
    let root = copy.lease.working_directory().unwrap();
    std::fs::write(root.join("main.tex"), after).unwrap();
    // An unexecuted second target keeps the apply interrupted, not completed.
    std::fs::write(root.join("z.tex"), "pending disk").unwrap();
    if mixed {
        std::fs::write(root.join("a.tex"), "added disk").unwrap();
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
        .begin_proposal_application(f.conversation, proposal, scope.operation, buffers())
        .await
        .unwrap();
    let observe = |version: EditorVersion, text: &str| EditorObservation {
        path: "main.tex".into(),
        version,
        text: text.into(),
    };
    let applied = EditorVersion {
        revision: Uuid::new_v4(),
        ..version.clone()
    };
    let restored = EditorVersion {
        revision: Uuid::new_v4(),
        ..version.clone()
    };
    let restore = Uuid::new_v4();
    if mixed {
        service
            .apply_disk_proposal_file(scope, "a.tex".into())
            .await
            .unwrap();
    }
    let wire_scope = serde_json::json!({"conversationId":scope.conversation,"manuscriptId":scope.manuscript,"proposalId":scope.proposal,"operationId":scope.operation});
    let read = serde_json::json!({"action":"read","scope":wire_scope,"path":"main.tex"});
    let initial = serde_json::to_value(
        boundary::execute(&f.store, serde_json::from_value(read.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(initial["restoration"], serde_json::Value::Null);
    for invalid in [
        serde_json::json!({"action":"read","scope":wire_scope,"path":"../main.tex"}),
        serde_json::json!({"action":"read","scope":wire_scope,"path":"z.tex"}),
        serde_json::json!({"action":"read","scope":{ "conversationId":Uuid::new_v4(),"manuscriptId":scope.manuscript,"proposalId":scope.proposal,"operationId":scope.operation},"path":"main.tex"}),
    ] {
        assert!(
            boundary::execute(&f.store, serde_json::from_value(invalid).unwrap())
                .await
                .is_err()
        );
    }
    assert!(
        service
            .prepare_editor_restoration(scope, restore, observe(applied.clone(), after))
            .await
            .is_err()
    );
    assert!(matches!(
        service
            .prepare_editor_proposal_file(scope, observe(version.clone(), before))
            .await
            .unwrap(),
        EditorAdmission::Dispatch(_)
    ));
    assert!(
        service
            .prepare_editor_restoration(scope, restore, observe(applied.clone(), after))
            .await
            .is_err()
    );
    service
        .confirm_editor_proposal_file(scope, observe(applied.clone(), after))
        .await
        .unwrap();
    assert!(service.rollback_proposal_remainder(scope).await.is_err());
    assert!(
        service
            .confirm_editor_restoration(scope, restore, observe(restored.clone(), before))
            .await
            .is_err()
    );
    for (identity, text) in [(version.clone(), after), (applied.clone(), "later input")] {
        assert!(
            service
                .prepare_editor_restoration(scope, restore, observe(identity, text))
                .await
                .is_err()
        );
    }
    let boundary::Response::Edit {schema_version,scope:returned_scope,restore_id,expected,text} = boundary::execute(&f.store,
        serde_json::from_value(serde_json::json!({"action":"prepare","scope":wire_scope,"restoreId":restore,"current":{"path":"main.tex","sessionId":applied.session_id,"revision":applied.revision,"text":after}})).unwrap()).await.unwrap()
    else {
        panic!("expected restore dispatch")
    };
    assert_eq!(schema_version, 1);
    assert_eq!(serde_json::to_value(returned_scope).unwrap(), wire_scope);
    let command = crate::agent::writing_submission::workspace::EditorDispatch {
        operation: restore_id,
        expected: observe(
            EditorVersion {
                session_id: expected.session_id,
                revision: expected.revision,
            },
            &expected.text,
        ),
        text,
    };
    assert_eq!(command.operation, restore);
    assert_ne!(command.operation, scope.operation);
    assert_eq!(command.expected.version, applied);
    assert_eq!(command.expected.text, after);
    assert_eq!(command.text, before);
    // Retrying preparation after the edit returns identity only, not another edit.
    assert!(matches!(
        service
            .prepare_editor_restoration(scope, restore, observe(restored.clone(), before))
            .await
            .unwrap(),
        EditorRestoreAdmission::Existing { .. }
    ));
    assert!(
        service
            .prepare_editor_restoration(scope, Uuid::new_v4(), observe(restored.clone(), before))
            .await
            .is_err()
    );
    for (id, identity, text) in [
        (Uuid::new_v4(), restored.clone(), before),
        (restore, applied.clone(), before),
        (
            restore,
            EditorVersion {
                session_id: Uuid::new_v4(),
                ..restored.clone()
            },
            before,
        ),
        (restore, restored.clone(), "未保存 baseline\n"),
    ] {
        assert!(
            service
                .confirm_editor_restoration(scope, id, observe(identity, text))
                .await
                .is_err()
        );
    }
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        // Simulate a late disk ACK after compensation admission. All files are
        // acknowledged, but finishing apply is no longer allowed.
        tx.execute("update agent_proposal_application_files set phase='dispatched' where relative_path='z.tex'",[]).unwrap();
        tx.execute("update agent_proposal_application_files set phase='acknowledged' where relative_path='z.tex'",[]).unwrap();
        assert!(applications::finish_in(&mut tx, scope).is_err());
        tx.rollback().unwrap();
    }
    f.store.connection().unwrap().execute_batch("create trigger fail_restore_phase before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .confirm_editor_restoration(scope, restore, observe(restored.clone(), before))
            .await
            .is_err()
    );
    {
        let mut db = f.store.connection().unwrap();
        // Even a repository caller committing after an error cannot retain half an ACK.
        let mut tx = db.transaction().unwrap();
        assert!(
            applications::acknowledge_editor_restored_in(
                &mut tx, scope, "main.tex", restore, &restored
            )
            .is_err()
        );
        tx.commit().unwrap();
        assert_eq!(
            applications::read_editor_restoration_in(&db, scope, "main.tex")
                .unwrap()
                .unwrap()
                .result_revision,
            None
        );
        db.execute_batch("drop trigger fail_restore_phase;")
            .unwrap();
    }
    let confirmed=serde_json::to_value(boundary::execute(&f.store,serde_json::from_value(serde_json::json!({"action":"confirm","scope":wire_scope,"restoreId":restore,"actual":{"path":"main.tex","sessionId":restored.session_id,"revision":restored.revision,"text":before}})).unwrap()).await.unwrap()).unwrap();
    assert_eq!(
        confirmed["application"]["status"],
        if mixed {
            "recoveryRequired"
        } else {
            "rolledBack"
        }
    );
    assert_eq!(
        confirmed["restoration"]["resultRevision"],
        restored.revision.to_string()
    );
    assert_eq!(
        serde_json::to_value(
            boundary::execute(&f.store, serde_json::from_value(read).unwrap())
                .await
                .unwrap()
        )
        .unwrap(),
        confirmed
    );
    if mixed {
        use crate::agent::writing_application_commands as commands;
        let rollback = || {
            serde_json::from_value(
                serde_json::json!({"action":"rollbackRemainder","scope":wire_scope}),
            )
            .unwrap()
        };
        let source = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/a.tex");
        std::fs::write(&source, "later user edit").unwrap();
        assert!(commands::execute(&f.store, rollback()).await.is_err());
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "later user edit");
        std::fs::write(&source, "added disk").unwrap();
        let done =
            serde_json::to_value(commands::execute(&f.store, rollback()).await.unwrap()).unwrap();
        assert_eq!(done["status"], "rolledBack");
        assert!(!source.exists());
        std::fs::write(&source, "new independent file").unwrap();
        assert_eq!(
            serde_json::to_value(commands::execute(&f.store, rollback()).await.unwrap()).unwrap(),
            done
        );
        assert_eq!(
            std::fs::read_to_string(&source).unwrap(),
            "new independent file"
        );
        assert!(service.rollback_disk_proposal(scope).await.is_err());
    }
    let saved = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(saved.status, ApplicationStatus::RolledBack);
    assert_eq!(
        saved
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .phase,
        ApplicationPhase::Restored
    );
    assert_eq!(
        saved
            .files
            .iter()
            .find(|f| f.path == "z.tex")
            .unwrap()
            .phase,
        ApplicationPhase::Pending
    );
    assert_eq!(
        service
            .confirm_editor_restoration(scope, restore, observe(restored.clone(), before))
            .await
            .unwrap(),
        saved
    );
    assert!(
        service
            .confirm_editor_restoration(
                scope,
                restore,
                observe(
                    EditorVersion {
                        revision: Uuid::new_v4(),
                        ..restored
                    },
                    before
                )
            )
            .await
            .is_err()
    );
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!source.join("z.tex").exists());
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select sha256 from manuscript_files where relative_path='main.tex'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        format!("{:x}", Sha256::digest(b"Frozen manuscript"))
    );
}
