//! The original selected-anchor writer preserves replay, rollback and recovery fencing.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from ai_evidence_bundles),(select count(*) from ai_evidence_items),(select count(*) from source_anchors)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_evidence_rolls_back_all_items_and_replays_frozen_context() {
    let (f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("create temp trigger reject_selection_item before insert on ai_evidence_items when new.ordinal=1 begin select raise(abort,'synthetic private evidence'); end;").unwrap();
    let error = f
        .service
        .persist_selection_task_evidence(f.store.clone(), key.clone(), run.id.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.to_string().contains("synthetic private evidence"));
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_selection_item")
        .unwrap();
    let bundle = f
        .service
        .persist_selection_task_evidence(f.store.clone(), key.clone(), run.id.clone())
        .await
        .unwrap();
    assert_eq!(counts(&f), (before.0 + 1, before.1 + 2, before.2));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update source_anchors set quote_exact='later text' where id='reference-anchor'",
            [],
        )
        .unwrap();
    let replay = f
        .service
        .persist_selection_task_evidence(f.store.clone(), key, run.id)
        .await
        .unwrap();
    let mut frozen = serde_json::to_value(bundle).unwrap();
    for item in frozen["items"].as_array_mut().unwrap() {
        assert_eq!(item["currentAnchorStatus"], "page_level");
        item["currentAnchorStatus"] = json!("stale");
    }
    assert_eq!(serde_json::to_value(replay).unwrap(), frozen);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_evidence_unknown_commit_or_rollback_fences_and_recovers() {
    for commit in [true, false] {
        let (f, key, credentials, draft) = setup();
        let run = admit(&f, &key, &credentials, draft).await;
        let before = counts(&f);
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger reject_selection_item before insert on ai_evidence_items begin select raise(abort,'synthetic private evidence'); end;").unwrap();
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
            .persist_selection_task_evidence(f.store.clone(), key, run.id.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&run.id));
        assert!(
            error
                .to_string()
                .contains(if commit { "commit" } else { "rollback" })
        );
        assert!(!error.to_string().contains("synthetic private evidence"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), before);
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
