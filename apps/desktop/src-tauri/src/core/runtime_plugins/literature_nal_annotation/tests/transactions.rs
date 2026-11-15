use super::*;
use rusqlite::hooks::TransactionOperation;

#[test]
fn rollback_and_commit_failures_fence_the_owner_and_cold_reopen_preserves_core_notes() {
    for fail_rollback in [true, false] {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let call = fixture.call("private-annotation-title");
        let mut db = fixture.db.lock().unwrap();
        if fail_rollback {
            db.execute_batch(
                "create temp trigger annotation_link_rejected before insert on nal_annotations
                 begin select raise(abort,'private-annotation-trigger-payload');end;",
            )
            .unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }));
        } else {
            db.commit_hook(Some(|| true));
        }
        let error = apply(
            &mut db,
            &fixture.resources,
            &call.captured,
            &call.input,
            &call.plan,
        )
        .unwrap_err();
        assert_eq!(error.outcome, AnnotationOutcome::Unknown);
        assert_eq!(
            error.to_string(),
            "笔记关联提交或回滚结果未知，禁止自动重试。"
        );
        if fail_rollback {
            assert!(!db.is_autocommit());
            assert_eq!(error.problem.phase, AnnotationPhase::LinkInsert);
            let AnnotationCause::Data(cause) = &error.problem.cause else {
                panic!("expected original link SQL failure")
            };
            assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
            assert_eq!(
                cause.sqlite_extended_code,
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER)
            );
            let rollback = error.rollback_error.as_ref().unwrap();
            assert_eq!(
                rollback.sqlite_code.as_deref(),
                Some("AuthorizationForStatementDenied")
            );
            assert_eq!(
                rollback.sqlite_extended_code,
                Some(rusqlite::ffi::SQLITE_AUTH)
            );
            db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        } else {
            // SQLite rolls this transaction back itself. Autocommit alone is
            // not a confirmed rollback receipt for the rejected COMMIT call.
            assert!(db.is_autocommit());
            assert_eq!(error.problem.phase, AnnotationPhase::Commit);
            let AnnotationCause::Database(cause) = &error.problem.cause else {
                panic!("expected original commit failure")
            };
            assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
            assert_eq!(
                cause.sqlite_extended_code,
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_COMMITHOOK)
            );
            assert!(error.rollback_error.is_none());
            db.commit_hook(None::<fn() -> bool>);
        }
        let diagnostic = serde_json::to_string(&error).unwrap();
        assert!(!diagnostic.contains("private-annotation-trigger-payload"));
        assert!(!diagnostic.contains("private-annotation-title"));
        assert!(!diagnostic.contains("create temp trigger"));
        assert_eq!(
            serde_json::to_value(&error.outcome).unwrap(),
            serde_json::json!({"status":"unknown"})
        );
        assert!(!diagnostic.contains("observedRevision"));
        drop(db);
        assert!(fixture.db.lock().is_err());
        assert_eq!(fixture.reopen_fenced().snapshot(), before);
    }
}
