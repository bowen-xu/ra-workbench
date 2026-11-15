use super::*;
use crate::agent::repository::proposal_applications::{self, ApplicationAdmission};

#[path = "application/progress.rs"]
mod progress;

#[tokio::test]
async fn application_read_reports_durable_progress_without_replaying_or_reading_sources() {
    use proposal_applications::{ApplicationPhase, ApplicationStatus, ApplicationTarget};
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    assert!(
        service
            .find_proposal_application(f.conversation, f.manuscript, proposal)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .find_proposal_application(Uuid::new_v4(), f.manuscript, proposal)
            .await
            .is_err()
    );
    service
        .begin_proposal_application(
            f.conversation,
            proposal,
            operation,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let initial = service
        .read_proposal_application(f.conversation, f.manuscript, proposal, operation)
        .await
        .unwrap();
    assert_eq!(initial.operation_id, operation);
    assert_eq!(initial.proposal_id, proposal);
    assert_eq!(initial.status, ApplicationStatus::Applying);
    assert_eq!(initial.files.len(), 1);
    assert_eq!(initial.files[0].path, "main.tex");
    assert_eq!(initial.files[0].target, ApplicationTarget::Disk);
    assert_eq!(initial.files[0].phase, ApplicationPhase::Pending);
    assert_eq!(
        service
            .find_proposal_application(f.conversation, f.manuscript, proposal)
            .await
            .unwrap(),
        Some(initial.clone())
    );
    // The journal remains readable after the source disappears. Reading does
    // not recreate files, dispatch tools, or turn an uncertain write into ACK.
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    std::fs::remove_file(&source).unwrap();
    {
        let db = f.store.connection().unwrap();
        db.execute("update agent_proposal_application_files set phase='dispatched' where application_id=?1", [operation.to_string()]).unwrap();
        db.execute(
            "update agent_proposal_applications set status='recovery_required' where id=?1",
            [operation.to_string()],
        )
        .unwrap();
    }
    let recovered = service
        .read_proposal_application(f.conversation, f.manuscript, proposal, operation)
        .await
        .unwrap();
    assert_eq!(recovered.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(recovered.files[0].phase, ApplicationPhase::Dispatched);
    // A new service instance can recover the operation ID from the proposal;
    // it does not need renderer state or readable source files.
    assert_eq!(
        WritingSubmissionService::from_app_store(&f.store)
            .find_proposal_application(f.conversation, f.manuscript, proposal)
            .await
            .unwrap(),
        Some(recovered.clone())
    );
    let wire = crate::agent::writing_application_commands::find(
        &f.store,
        crate::agent::writing_proposal_commands::ProposalRequest {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            proposal_id: proposal,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let wire = serde_json::to_value(wire).unwrap();
    assert_eq!(wire["scope"]["operationId"], operation.to_string());
    assert_eq!(wire["status"], "recoveryRequired");
    assert_eq!(
        service
            .read_proposal_application(f.conversation, f.manuscript, proposal, operation)
            .await
            .unwrap(),
        recovered
    );
    assert!(!source.exists());
    for (conversation, manuscript, proposal, operation) in [
        (Uuid::new_v4(), f.manuscript, proposal, operation),
        (f.conversation, Uuid::new_v4(), proposal, operation),
        (f.conversation, f.manuscript, Uuid::new_v4(), operation),
        (f.conversation, f.manuscript, proposal, Uuid::new_v4()),
    ] {
        assert!(
            service
                .read_proposal_application(conversation, manuscript, proposal, operation)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn application_admission_records_complete_intent_once_without_dispatching() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    assert_eq!(
        service
            .begin_proposal_application(f.conversation, proposal, operation, request())
            .await
            .unwrap(),
        ApplicationAdmission::Created(operation)
    );
    assert_eq!(
        service
            .begin_proposal_application(f.conversation, proposal, operation, request())
            .await
            .unwrap(),
        ApplicationAdmission::Existing(operation)
    );
    assert!(
        service
            .begin_proposal_application(f.conversation, proposal, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    assert!(
        service
            .begin_proposal_application(Uuid::new_v4(), proposal, operation, request())
            .await
            .is_err()
    );
    assert!(
        service
            .begin_proposal_application(f.conversation, Uuid::new_v4(), operation, request())
            .await
            .is_err()
    );
    assert!(
        service
            .reject_proposal(f.conversation, f.manuscript, proposal)
            .await
            .is_err()
    );
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    assert_eq!(std::fs::read(&source).unwrap(), b"Frozen manuscript");
    // Existing means reconcile, even when live sources no longer pass preflight.
    std::fs::write(source, b"later user edit").unwrap();
    assert_eq!(
        service
            .begin_proposal_application(f.conversation, proposal, operation, request())
            .await
            .unwrap(),
        ApplicationAdmission::Existing(operation)
    );
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select status from agent_change_sets where id=?1",
            [proposal.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "applying"
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_proposal_applications",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let target: (String, String) = db
        .query_row(
            "select target_kind,phase from agent_proposal_application_files",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(target, ("disk".into(), "pending".into()));
}

#[tokio::test]
async fn recovery_pending_application_revalidates_and_resumes_same_operation() {
    use proposal_applications::{ApplicationPhase, ApplicationStatus};
    let f = fixture_with_objects();
    let scope = super::apply_disk::existing(&f, false).await;
    {
        let mut db = f.store.connection().unwrap();
        assert_eq!(
            proposal_applications::recover_on_startup(&mut db).unwrap(),
            1
        );
    }
    let service = WritingSubmissionService::from_app_store(&f.store);
    let resumed = service
        .resume_proposal_application(
            scope,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ApplicationStatus::Applying);
    assert_eq!(resumed.files[0].phase, ApplicationPhase::Pending);
    let applied = service
        .apply_disk_proposal_file(scope, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(applied.status, ApplicationStatus::Applied);
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex"),
        )
        .unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn recovery_resume_rejects_any_already_dispatched_file() {
    use proposal_applications::{ApplicationPhase, ApplicationStatus};
    let f = fixture_with_objects();
    let scope = super::apply_disk::existing(&f, false).await;
    {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            proposal_applications::dispatch_in(&tx, scope, "main.tex").unwrap(),
            proposal_applications::DispatchAdmission::NewIntent
        );
        tx.commit().unwrap();
        assert_eq!(
            proposal_applications::recover_on_startup(&mut db).unwrap(),
            1
        );
    }
    let service = WritingSubmissionService::from_app_store(&f.store);
    assert!(
        service
            .resume_proposal_application(
                scope,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![],
                },
            )
            .await
            .is_err()
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
    assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(saved.files[0].phase, ApplicationPhase::Dispatched);
}

#[tokio::test]
async fn stale_admission_never_leaves_a_journal_or_changes_proposal_status() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    std::fs::write(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
        b"changed",
    )
    .unwrap();
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
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_proposal_applications",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select status from agent_change_sets", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}

#[tokio::test]
async fn admission_savepoint_rolls_back_all_rows_even_if_caller_commits_after_failure() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let mut db = f.store.connection().unwrap();
    db.execute_batch("create trigger fail_admission before update of status on agent_change_sets when new.status='applying' begin select raise(abort,'fixture'); end;").unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(
        proposal_applications::begin_in(
            &mut tx,
            f.conversation,
            f.manuscript,
            proposal,
            Uuid::new_v4()
        )
        .is_err()
    );
    tx.commit().unwrap();
    for table in [
        "agent_proposal_applications",
        "agent_proposal_application_files",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("select status from agent_change_sets", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}
