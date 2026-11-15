//! Real package publication uses the original SQL, rollback and recovery owner.
use super::*;
use crate::ai::{generation_support::AiRun, paper_reading_contracts::PaperAiReadingContextItem};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::collections::BTreeMap;

async fn fixture() -> (Fixture, AiRun, Vec<PaperAiReadingContextItem>) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, input) = analysis_overview::seeded_task(&f);
    let items = crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap()
    .context_items;
    f.bind_actual_package(catalog, key.clone());
    let mut task = f
        .service
        .build_analysis_overview_task(&key, input)
        .await
        .unwrap();
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, task.clone())
        .await
        .unwrap();
    task.confirmed_scope_hash = Some(preview.scope_hash);
    task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = f
        .service
        .admit_analysis_overview(f.store.clone(), credentials, key, task, None)
        .await
        .unwrap();
    (f, run, items)
}
fn counts(f: &Fixture, run: &str) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row(
        "select (select count(*) from ai_evidence_bundles where run_id=?1),
         (select count(*) from source_anchors where id like 'docling:' || ?1 || ':%'),
         (select count(*) from ai_docling_evidence_context_items c join ai_evidence_items e on e.id=c.evidence_item_id join ai_evidence_bundles b on b.id=e.bundle_id where b.run_id=?1)",
        [run], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_docling_storage_rolls_back_all_rows_retries_and_reopens_original_bundle() {
    let (f, run, items) = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger reject_docling_context before insert on ai_docling_evidence_context_items begin select raise(abort,'private synthetic document must not escape'); end;").unwrap();
    let failure = f
        .service
        .persist_analysis_docling_evidence(
            f.store.clone(),
            f.key.clone(),
            run.clone(),
            items.clone(),
            BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "storage_error");
    assert!(!failure.to_string().contains("private synthetic document"));
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(counts(&f, &run.id), (0, 0, 0));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_docling_context")
        .unwrap();
    let bundle = f
        .service
        .persist_analysis_docling_evidence(
            f.store.clone(),
            f.key.clone(),
            run.clone(),
            items,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(counts(&f, &run.id), (1, 3, 3));
    let replay = f
        .service
        .persist_analysis_docling_evidence(
            f.store.clone(),
            f.key.clone(),
            run.clone(),
            Vec::new(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&bundle).unwrap()
    );
    let f = reopen_actual_fixture(f, false).await;
    let restored = f
        .service
        .persist_analysis_docling_evidence(
            f.store.clone(),
            f.key.clone(),
            run.clone(),
            Vec::new(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(bundle).unwrap()
    );
    assert_eq!(counts(&f, &run.id), (1, 3, 3));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_docling_storage_unknown_commit_or_rollback_fences_and_reopens_without_rows()
{
    for fail_commit in [true, false] {
        let (mut f, run, items) = fixture().await;
        f.service.shutdown().await.unwrap();
        let (catalog, key) = actual_package();
        f.bind_actual_package(catalog, key);
        if fail_commit {
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
        let failure = f
            .service
            .persist_analysis_docling_evidence(
                f.store.clone(),
                f.key.clone(),
                run.clone(),
                items,
                BTreeMap::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code(), "data_operation_outcome_unknown");
        assert!(failure.to_string().contains(&run.id));
        assert!(failure.to_string().contains("evidence_bundle"));
        assert!(
            failure
                .to_string()
                .contains(if fail_commit { "commit" } else { "rollback" })
        );
        assert!(f.store.connection().is_err());
        if !fail_commit {
            assert!(
                f.service.host.lock().await.is_none(),
                "an unconfirmed rollback must not start Native"
            );
        }
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f, &run.id), (0, 0, 0));
        f.service.shutdown().await.unwrap();
    }
}
