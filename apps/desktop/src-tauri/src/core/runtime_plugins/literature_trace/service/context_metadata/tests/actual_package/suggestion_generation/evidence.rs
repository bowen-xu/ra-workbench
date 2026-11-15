//! Original page evidence publication and Core recovery fence, with the actual host.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select (select count(*) from ai_evidence_bundles),
         (select count(*) from ai_evidence_items),
         (select count(*) from source_anchors where attachment_id='selected')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_pages_roll_back_anchor_bundle_and_items_then_replay() {
    let (f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("create temp trigger reject_page_evidence before insert on ai_evidence_items begin select raise(abort,'synthetic private page body'); end;").unwrap();
    let error = f
        .service
        .persist_page_task_evidence(f.store.clone(), key.clone(), run.id.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.to_string().contains("synthetic private page body"));
    assert_eq!(counts(&f), before);
    assert!(f.store.connection().unwrap().is_autocommit());
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_page_evidence")
        .unwrap();
    let bundle = f
        .service
        .persist_page_task_evidence(f.store.clone(), key.clone(), run.id.clone())
        .await
        .unwrap();
    assert_eq!(counts(&f), (before.0 + 1, before.1 + 1, before.2 + 1));
    // A saved bundle remains frozen when the live page index later changes.
    f.store.connection().unwrap().execute("update attachment_page_texts set page_text='Later content' where attachment_id='selected'", []).unwrap();
    let replay = f
        .service
        .persist_page_task_evidence(f.store.clone(), key.clone(), run.id)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&bundle).unwrap()
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_pages_unknown_commit_or_rollback_fences_and_recovers() {
    for commit in [true, false] {
        let (f, key, credentials, draft) = setup();
        let run = admit(&f, &key, &credentials, draft).await;
        let before = counts(&f);
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger reject_page_evidence before insert on ai_evidence_items begin select raise(abort,'synthetic private page body'); end;").unwrap();
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
            .persist_page_task_evidence(f.store.clone(), key, run.id.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&run.id));
        assert!(error.to_string().contains("evidence_bundle"));
        assert!(
            error
                .to_string()
                .contains(if commit { "commit" } else { "rollback" })
        );
        assert!(!error.to_string().contains("synthetic private page body"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), before);
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_generation_preserves_unknown_evidence_through_core_stage_failure()
 {
    let (f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let signal = CancellationSignal::default();
    let (host, wire) = host::Host::new(
        &f,
        credentials,
        run.clone(),
        signal.clone(),
        payload("concept_suggestion_v1"),
    );
    host.page_commit_unknown.store(true, Ordering::SeqCst);
    let error = execute_suggestions(&host, &run, &signal).await.unwrap_err();
    assert_eq!(error.code(), "data_operation_outcome_unknown");
    assert!(error.to_string().contains(&run.id));
    assert!(error.to_string().contains("evidence_bundle"));
    assert!(error.to_string().contains("commit"));
    assert!(f.store.connection().is_err());
    assert_eq!(wire.calls.load(Ordering::SeqCst), 0);
    drop(host);
    let f = reopen_actual_fixture(f, true).await;
    assert_eq!(counts(&f).0, 0);
    let db = f.store.connection().unwrap();
    assert_eq!(db.query_row("select count(*) from ai_run_stages where run_id=?1 and error_code='data_operation_outcome_unknown'",[&run.id],|row|row.get::<_,i64>(0)).unwrap(),0);
    drop(db);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
