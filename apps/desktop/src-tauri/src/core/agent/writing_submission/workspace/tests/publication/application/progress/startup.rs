use super::*;

#[tokio::test]
async fn service_startup_keeps_completed_application_terminal() {
    let (f, scope, receipt) = admitted().await;
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        dispatch_in(&tx, scope, &receipt.path).unwrap();
        acknowledge_in(&tx, scope, &receipt).unwrap();
        finish_in(&mut tx, scope).unwrap();
        tx.commit().unwrap();
    }
    let _service = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let db = f.store.connection().unwrap();
    assert_eq!(
        read_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation
        )
        .unwrap()
        .status,
        ApplicationStatus::Applied
    );
}

#[tokio::test]
async fn startup_retires_owner_without_replaying_or_fabricating_receipts() {
    for phase in [
        ApplicationPhase::Pending,
        ApplicationPhase::Dispatched,
        ApplicationPhase::Acknowledged,
    ] {
        let (f, scope, receipt) = admitted().await;
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        if phase != ApplicationPhase::Pending {
            dispatch_in(&tx, scope, &receipt.path).unwrap();
        }
        if phase == ApplicationPhase::Acknowledged {
            acknowledge_in(&tx, scope, &receipt).unwrap();
        }
        tx.commit().unwrap();
        assert_eq!(
            proposal_applications::recover_on_startup(&mut db).unwrap(),
            1
        );
        assert_eq!(
            proposal_applications::recover_on_startup(&mut db).unwrap(),
            0
        );
        let saved = read_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .unwrap();
        assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
        assert_eq!(saved.files[0].phase, phase);
        let tx = db.transaction().unwrap();
        if phase == ApplicationPhase::Pending {
            assert!(dispatch_in(&tx, scope, &receipt.path).is_err());
        }
        drop(tx);
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
}

#[tokio::test]
async fn startup_failure_preserves_application_and_proposal_for_retry() {
    let (f, scope, _) = admitted().await;
    let mut db = f.store.connection().unwrap();
    db.execute_batch("create trigger deny_recovery before update of status on agent_change_sets when new.status='recovery_required' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(proposal_applications::recover_on_startup(&mut db).is_err());
    let saved = read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(saved.status, ApplicationStatus::Applying);
    db.execute_batch("drop trigger deny_recovery").unwrap();
    assert_eq!(
        proposal_applications::recover_on_startup(&mut db).unwrap(),
        1
    );
}
