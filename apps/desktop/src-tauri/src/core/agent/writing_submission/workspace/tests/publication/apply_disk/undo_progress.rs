use super::*;
use crate::agent::repository::proposal_undos;

#[tokio::test]
async fn undo_start_rechecks_all_sources_and_defer_blocks_pending_dispatch() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    let operation = Uuid::new_v4();
    service
        .prepare_proposal_undo(original, operation, request())
        .await
        .unwrap();
    std::fs::write(source(&f).join("main.tex"), b"later input").unwrap();
    assert!(
        service
            .start_proposal_undo(original, operation, request())
            .await
            .is_err()
    );
    assert_eq!(
        service
            .read_proposal_undo(original, operation)
            .await
            .unwrap()
            .unwrap()
            .application
            .status,
        ApplicationStatus::Prepared
    );
    std::fs::write(source(&f).join("main.tex"), b"replacement").unwrap();
    service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    let deferred = service
        .defer_proposal_undo(original, operation)
        .await
        .unwrap();
    assert_eq!(
        deferred.application.status,
        ApplicationStatus::RecoveryRequired
    );
    assert_eq!(
        deferred.application.files[0].phase,
        ApplicationPhase::Pending
    );
    assert_eq!(
        service
            .defer_proposal_undo(original, operation)
            .await
            .unwrap()
            .application,
        deferred.application
    );
    assert_eq!(
        service
            .start_proposal_undo(original, operation, request())
            .await
            .unwrap()
            .application,
        deferred.application
    );
    let mut db = f.store.connection().unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(proposal_undos::dispatch_in(&tx, original, operation, "main.tex").is_err());
    assert!(proposal_undos::finish_in(&mut tx, original, operation).is_err());
    drop(tx);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
}
