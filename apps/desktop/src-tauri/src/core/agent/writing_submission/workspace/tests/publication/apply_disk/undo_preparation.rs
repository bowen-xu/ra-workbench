use super::*;
use crate::agent::repository::proposal_undos;

#[tokio::test]
async fn prepared_undo_survives_reopen_and_retry_without_new_baseline_or_effects() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    let undo = Uuid::new_v4();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    assert!(
        service
            .prepare_proposal_undo(original, undo, request())
            .await
            .is_err()
    );
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    assert!(
        service
            .read_proposal_undo(original, undo)
            .await
            .unwrap()
            .is_none()
    );
    let prepared = service
        .prepare_proposal_undo(original, undo, request())
        .await
        .unwrap();
    assert_eq!(prepared.original_operation, original.operation);
    assert_eq!(prepared.application.status, ApplicationStatus::Prepared);
    let _sessions = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    assert_eq!(
        service
            .read_proposal_undo(original, undo)
            .await
            .unwrap()
            .unwrap()
            .application
            .status,
        ApplicationStatus::Prepared
    );
    assert_eq!(
        prepared.application.files[0].phase,
        ApplicationPhase::Pending
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    // Even with a valid unchanged source, a second unresolved undo is refused.
    assert!(
        service
            .prepare_proposal_undo(original, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    // A lost response must not substitute a newer source revision on retry.
    std::fs::write(source(&f).join("main.tex"), b"later input").unwrap();
    let retried = service
        .prepare_proposal_undo(original, undo, request())
        .await
        .unwrap();
    assert_eq!(retried.application, prepared.application);
    assert_eq!(retried.changes, prepared.changes);
    assert!(
        service
            .prepare_proposal_undo(original, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    let db_path = f.root.path().join("undo.sqlite");
    {
        let db = f.store.connection().unwrap();
        assert!(
            db.execute(
                "update agent_proposal_undo_sources set source_json=null where operation_id=?1",
                [undo.to_string()]
            )
            .is_err()
        );
        assert!(
            db.execute(
                "delete from agent_proposal_undos where operation_id=?1",
                [undo.to_string()]
            )
            .is_err()
        );
        db.execute("vacuum into ?1", [db_path.to_str().unwrap()])
            .unwrap();
    }
    let db = rusqlite::Connection::open(db_path).unwrap();
    let recovered = proposal_undos::read_in(&db, original, undo)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.application, prepared.application);
    assert_eq!(recovered.changes, prepared.changes);
    assert!(
        proposal_undos::read_in(
            &db,
            ApplicationScope {
                conversation: Uuid::new_v4(),
                ..original
            },
            undo
        )
        .is_err()
    );
    assert!(proposal_undos::read_in(&db, original, original.operation).is_err());
    assert!(
        proposal_undos::read_in(
            &db,
            ApplicationScope {
                proposal: Uuid::new_v4(),
                ..original
            },
            Uuid::new_v4()
        )
        .is_err()
    );
    let saved = service
        .read_proposal_application(
            original.conversation,
            original.manuscript,
            original.proposal,
            original.operation,
        )
        .await
        .unwrap();
    assert_eq!(saved.status, ApplicationStatus::Applied);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"later input"
    );
}

#[tokio::test]
async fn undo_admission_failure_rolls_back_every_row_even_if_outer_commit_continues() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, true).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let changes = service
        .preview_proposal_undo(
            original,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let undo = Uuid::new_v4();
    let mut db = f.store.connection().unwrap();
    db.execute_batch("create trigger deny_undo_source before insert on agent_proposal_undo_sources begin select raise(abort,'fixture'); end;").unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(proposal_undos::begin_in(&mut tx, original, undo, &changes).is_err());
    tx.commit().unwrap();
    assert!(
        proposal_undos::read_in(&db, original, undo)
            .unwrap()
            .is_none()
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
    db.execute_batch("drop trigger deny_undo_source").unwrap();
    let mut tx = db.transaction().unwrap();
    let mut invalid = changes.clone();
    invalid[0].after = None;
    assert!(proposal_undos::begin_in(&mut tx, original, undo, &invalid).is_err());
    let prepared = proposal_undos::begin_in(&mut tx, original, undo, &changes).unwrap();
    assert!(prepared.changes[0].source.is_none());
    assert!(prepared.changes[0].before.is_none());
    assert!(prepared.changes[0].after.is_some());
    tx.commit().unwrap();
    assert!(!source(&f).join("main.tex").exists());
}
