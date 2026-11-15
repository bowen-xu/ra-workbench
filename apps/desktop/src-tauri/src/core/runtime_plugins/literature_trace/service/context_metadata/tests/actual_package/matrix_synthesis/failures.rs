//! Original evidence and promotion transactions fail atomically and retain recoverable identity.
use super::*;
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_evidence_failure_rolls_back_and_replays_frozen_bundle() {
    let (f, credentials, task) = setup().await;
    let run = admit(&f, &credentials, task).await;
    f.store.connection().unwrap().execute_batch("create temp trigger fail_matrix_evidence before insert on ai_evidence_items when new.ordinal=1 begin select raise(abort,'synthetic private evidence'); end;").unwrap();
    let error = f
        .service
        .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), run.id.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.to_string().contains("synthetic private evidence"));
    let counts: (i64,i64) = f.store.connection().unwrap().query_row("select (select count(*) from ai_evidence_bundles),(select count(*) from ai_evidence_items)", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_matrix_evidence")
        .unwrap();
    let bundle = f
        .service
        .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), run.id.clone())
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update source_anchors set quote_exact='Later source' where id='matrix-first-anchor'",
            [],
        )
        .unwrap();
    let replay = f
        .service
        .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), run.id)
        .await
        .unwrap();
    let mut expected = bundle;
    expected.items[0].current_anchor_status = "stale".into();
    assert_eq!(expected, replay);
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_promotion_failures_roll_back_view_revision_and_review() {
    for table in [
        "derived_research_views",
        "derived_research_view_revisions",
        "ai_review_decisions",
        "ai_candidates",
    ] {
        let (f, candidate) = generated().await;
        let candidate = qualify(&f, &candidate).await;
        let event = if table == "ai_candidates" {
            "update of review_state"
        } else {
            "insert"
        };
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_matrix_promotion before {event} on {table} begin select raise(abort,'synthetic private promotion'); end;")).unwrap();
        let request = review(&candidate, "accept_unchanged");
        let error = f
            .service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), request.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "storage_error");
        assert!(!error.to_string().contains("synthetic private promotion"));
        let counts:(i64,i64,i64)=f.store.connection().unwrap().query_row("select (select count(*) from derived_research_views),(select count(*) from derived_research_view_revisions),(select count(*) from ai_review_decisions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(counts, (0, 0, 0));
        let retained = crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap();
        assert_eq!(retained, candidate);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_matrix_promotion")
            .unwrap();
        f.service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), request)
            .await
            .unwrap();
        close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_unknown_evidence_commit_or_rollback_fences_and_reopens() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for commit in [true, false] {
        let (f, credentials, task) = setup().await;
        let run = admit(&f, &credentials, task).await;
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger fail_matrix_evidence before insert on ai_evidence_items begin select raise(abort,'synthetic private evidence'); end;").unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Transaction {
                        operation: TransactionOperation::Rollback
                    }
                ) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }));
        }
        let error = f
            .service
            .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), run.id.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&run.id));
        assert!(!error.to_string().contains("synthetic private evidence"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from ai_evidence_bundles", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_unknown_promotion_commit_or_rollback_fences_and_reopens() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for commit in [true, false] {
        let (f, candidate) = generated().await;
        let candidate = qualify(&f, &candidate).await;
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger fail_matrix_promotion before insert on ai_review_decisions begin select raise(abort,'synthetic private promotion'); end;").unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Transaction {
                        operation: TransactionOperation::Rollback
                    }
                ) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }));
        }
        let error = f
            .service
            .decide_analysis_candidate(
                f.store.clone(),
                f.key.clone(),
                review(&candidate, "accept_unchanged"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&candidate.id));
        assert!(!error.to_string().contains("synthetic private promotion"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        let counts:(i64,i64,i64) = f.store.connection().unwrap().query_row("select (select count(*) from derived_research_views),(select count(*) from derived_research_view_revisions),(select count(*) from ai_review_decisions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(counts, (0, 0, 0));
        assert_eq!(
            crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
        close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_changed_attachment_or_frozen_hash_rejects_before_publication()
 {
    for changed_attachment in [true, false] {
        let (f, credentials, draft) = setup().await;
        let run = admit(&f, &credentials, draft).await;
        if changed_attachment {
            f.store
                .connection()
                .unwrap()
                .execute(
                    "update attachments set content_sha256=?1 where id='matrix-first-pdf'",
                    ["a".repeat(64)],
                )
                .unwrap();
        } else {
            f.store.connection().unwrap().execute("update ai_scope_items set content_sha256=?1 where run_id=?2 and object_id='matrix-first-anchor'",rusqlite::params!["b".repeat(64),run.id]).unwrap();
        }
        let error = f
            .service
            .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), run.id)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_input");
        assert!(error.to_string().contains(if changed_attachment {
            "原文锚点或附件版本不可用"
        } else {
            "来源附件版本已变化"
        }));
        let count:i64=f.store.connection().unwrap().query_row("select (select count(*) from ai_evidence_bundles)+(select count(*) from ai_evidence_items)",[],|row|row.get(0)).unwrap();
        assert_eq!(count, 0);
        close(f).await;
    }
}
