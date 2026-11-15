//! Original confirmation SQL and unknown settlement against the independent package.
use super::{lifecycle::*, *};
use crate::ai::{error::AiError, review_support::ReviewFacts};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn setup() -> Fixture {
    let mut f = Fixture::new();
    seed(&f);
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    f
}
fn diagnostic(error: AiError) -> Value {
    serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap()
}
fn assert_sql_rollback(error: AiError) {
    assert_eq!(error.code(), "storage_error");
    let wire = diagnostic(error);
    assert!(!wire.to_string().contains("synthetic private confirmation"));
    assert_eq!(
        wire["nativeFailure"]["paperConfirmation"]["outcome"],
        "rolledBack"
    );
    assert_eq!(
        wire["nativeFailure"]["paperConfirmation"]["databaseError"]["sqliteCode"],
        "ConstraintViolation"
    );
    assert_eq!(
        wire["nativeFailure"]["paperConfirmation"]["databaseError"]["sqliteExtendedCode"],
        1811
    );
    assert!(
        wire["nativeFailure"]["requestId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_confirmation_read_failure_preserves_safe_sql_cause_before_transaction() {
    let f = setup();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "ai_candidates",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = execute(&f, waits::request())
        .await
        .err()
        .expect("denied candidate read");
    assert_eq!(error.code(), "storage_error");
    let wire = diagnostic(error);
    assert_eq!(
        wire["nativeFailure"]["paperConfirmation"]["outcome"],
        "notStarted"
    );
    assert!(
        wire["nativeFailure"]["paperConfirmation"]["databaseError"]["sqliteExtendedCode"]
            .is_number()
    );
    assert!(!wire.to_string().contains("select "));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(waits::counts(&f), (0, 0, 0, 0));
    execute(&f, waits::request()).await.unwrap();
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_confirmation_each_write_failure_rolls_back_both_fields_and_retries() {
    for (table, event, condition) in [
        ("ai_overview_confirmation_batches", "insert", ""),
        ("source_anchors", "insert", ""),
        (
            "structured_reading_units",
            "insert",
            "when new.unit_kind='conclusion'",
        ),
        ("reading_unit_evidence_links", "insert", ""),
        ("research_object_revisions", "insert", ""),
        (
            "ai_overview_field_confirmations",
            "insert",
            "when new.field_kind='conclusion'",
        ),
        ("ai_candidates", "update of review_state", ""),
    ] {
        let f = setup();
        let before = ReviewFacts::new(&f.store.connection().unwrap())
            .candidate("legacy-candidate")
            .unwrap();
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_confirmation before {event} on {table} {condition} begin select raise(abort,'synthetic private confirmation'); end;")).unwrap();
        assert_sql_rollback(
            execute(&f, waits::request())
                .await
                .err()
                .expect("injected SQL failure"),
        );
        assert_eq!(waits::counts(&f), (0, 0, 0, 0));
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate("legacy-candidate")
                .unwrap(),
            before
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_confirmation")
            .unwrap();
        let result = overview(execute(&f, waits::request()).await.unwrap());
        assert_eq!(result.batch.field_confirmations.len(), 2);
        assert_eq!(waits::counts(&f), (2, 1, 2, 1));
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_confirmation_unknown_commit_or_rollback_fences_and_recovers_same_key() {
    for commit in [true, false] {
        let f = setup();
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            f.store
                .connection()
                .unwrap()
                .authorizer(Some(|context: AuthContext<'_>| {
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
        let error = execute(&f, waits::request())
            .await
            .err()
            .expect("unconfirmed transaction");
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        let wire = diagnostic(error);
        assert_eq!(
            wire["nativeFailure"]["paperConfirmation"]["outcome"],
            "unknown"
        );
        assert!(
            wire["nativeFailure"]["paperConfirmation"]["objectId"]
                .as_str()
                .is_some()
        );
        assert_eq!(wire["nativeFailure"]["packageKey"], f.key);
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(waits::counts(&f), (0, 0, 0, 0));
        let result = overview(execute(&f, waits::request()).await.unwrap());
        assert_eq!(result.batch.field_confirmations.len(), 2);
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_confirmation_undo_and_marker_reset_failures_preserve_units_receipts_and_state()
 {
    for reset in [false, true] {
        let f = setup();
        let confirmed = overview(execute(&f, waits::request()).await.unwrap());
        let before = serde_json::to_value(&confirmed).unwrap();
        let table = if reset {
            "ai_overview_field_review_resets"
        } else {
            "ai_overview_confirmation_undos"
        };
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_confirmation before insert on {table} begin select raise(abort,'synthetic private confirmation'); end;")).unwrap();
        let request = Request::UndoOverview {
            draft: AiOverviewConfirmationUndoDraft {
                batch_id: confirmed.batch.id.clone(),
                field_kind: reset.then(|| "approach".into()),
                idempotency_key: "fault-undo-key".into(),
            },
        };
        assert_sql_rollback(
            execute(&f, request.clone())
                .await
                .err()
                .expect("injected undo failure"),
        );
        let Request::ConfirmOverview { draft } = waits::request() else {
            unreachable!()
        };
        let current = overview(
            execute(&f, Request::ConfirmOverview { draft })
                .await
                .unwrap(),
        );
        assert_eq!(serde_json::to_value(current).unwrap(), before);
        for field in &confirmed.batch.field_confirmations {
            assert_eq!(
                body(&f, &field.target_reading_unit_id).1,
                field.target_applied_revision
            );
        }
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_confirmation")
            .unwrap();
        let undone = overview(execute(&f, request).await.unwrap());
        if reset {
            assert!(undone.batch.active);
            for field in &confirmed.batch.field_confirmations {
                assert_eq!(
                    body(&f, &field.target_reading_unit_id).1,
                    field.target_applied_revision
                );
            }
        } else {
            assert!(!undone.batch.active);
        }
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_confirmation_sql_failure_retains_other_modules_and_same_key_retry() {
    let f = setup();
    f.store.connection().unwrap().execute("insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at) values('paper','title','crossref',0,'old')", []).unwrap();
    f.store.connection().unwrap().execute_batch("create temp trigger fail_confirmation before insert on ai_overview_field_confirmations when new.field_kind='conclusion' begin select raise(abort,'synthetic private confirmation'); end;").unwrap();
    let request = Request::ConfirmPaper {
        draft: AiPaperReviewConfirmationDraft {
            paper_id: "paper".into(),
            project_id: None,
            metadata_field_names: vec!["title".into()],
            overview_targets: vec![AiPaperReviewOverviewTargetDraft {
                candidate_id: "legacy-candidate".into(),
                candidate_revision: 1,
                field_kinds: vec!["approach".into(), "conclusion".into()],
            }],
            idempotency_key: "paper-partial-sql".into(),
        },
    };
    let first = paper(execute(&f, request.clone()).await.unwrap());
    assert_eq!(first.module_failures.len(), 1);
    assert!(first.module_failures[0].contains("1811"));
    assert!(!first.module_failures[0].contains("synthetic private confirmation"));
    assert_eq!(first.batch.metadata_confirmations.len(), 1);
    assert!(first.batch.overview_batch_ids.is_empty());
    assert_eq!(waits::counts(&f), (0, 0, 0, 0));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_confirmation")
        .unwrap();
    let repeated = paper(execute(&f, request).await.unwrap());
    assert_eq!(repeated.batch.id, first.batch.id);
    assert!(repeated.module_failures.is_empty());
    assert_eq!(repeated.batch.metadata_confirmations.len(), 1);
    assert_eq!(repeated.batch.overview_batch_ids.len(), 1);
    f.service.shutdown().await.unwrap();
}
