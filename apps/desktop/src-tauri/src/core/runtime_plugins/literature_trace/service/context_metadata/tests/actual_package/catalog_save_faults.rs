//! Real compiled package, Core-owned commit and process-level result loss.
use super::*;
use crate::runtime_plugins::literature_catalog::save::SaveOutcome;
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use research_workbench_native_sdk::{
    catalog::{CatalogItem, CatalogRequest},
    catalog_save::CatalogDraft,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::atomic::{AtomicBool, Ordering};

async fn fixture() -> Fixture {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    f.service
        .catalog(
            f.store.shared_agent_connection(),
            f.key.clone(),
            CatalogRequest::List { query: None },
        )
        .await
        .unwrap();
    f
}

fn draft() -> CatalogDraft {
    serde_json::from_value(json!({
        "id": null, "itemType": "journalArticle", "title": "C07 actual catalog save",
        "issuedYear": 2026, "creators": [{"familyName": "Example", "givenName": "Ada"}],
        "tags": ["synthetic"], "sourceKind": "user"
    }))
    .unwrap()
}

async fn save(f: &Fixture) -> crate::runtime_plugins::native::NativeResult<CatalogItem> {
    f.service
        .save_catalog(f.store.shared_agent_connection(), f.key.clone(), draft())
        .await
}

async fn identity(f: &Fixture) -> (u32, String) {
    let guard = f.service.host.lock().await;
    let host = guard.as_ref().unwrap();
    (
        host.process_id_for_test(),
        host.request_identity().unwrap().0,
    )
}

fn terminate(pid: u32) {
    assert!(pid > 1);
    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGKILL).unwrap();
}

fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select (select count(*) from bibliographic_items),
                (select count(*) from creators),
                (select count(*) from item_creators),
                (select count(*) from literature_search)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_catalog_host_exit_before_preparation_writes_nothing_and_recovers() {
    let f = fixture().await;
    let before = counts(&f);
    let (pid, instance) = identity(&f).await;
    terminate(pid);
    let error = save(&f).await.unwrap_err();
    assert!(matches!(
        error.code.as_str(),
        "plugin_host_input_failed" | "plugin_host_read_failed" | "plugin_host_exited"
    ));
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(error.instance_id.as_deref(), Some(instance.as_str()));
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert!(error.request_id.is_some());
    assert!(error.cleanup_error.is_none(), "{error:?}");
    assert_eq!(counts(&f), before);
    assert!(f.service.host.lock().await.is_none());

    let saved = save(&f).await.unwrap();
    assert_ne!(identity(&f).await.1, instance);
    assert_eq!(counts(&f).0, before.0 + 1);
    let after = counts(&f);
    let f = reopen_actual_fixture(f, false).await;
    let rows = f
        .service
        .catalog(
            f.store.shared_agent_connection(),
            f.key.clone(),
            CatalogRequest::Get {
                id: saved.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(rows, vec![saved]);
    assert_eq!(counts(&f), after);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_catalog_host_exit_after_commit_retains_identity_without_replay() {
    let f = fixture().await;
    let before = counts(&f);
    let (pid, instance) = identity(&f).await;
    let committed = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    {
        let after_commit = Arc::clone(&committed);
        let once = Arc::clone(&terminated);
        let db = f.store.connection().unwrap();
        db.commit_hook(Some(move || {
            after_commit.store(true, Ordering::SeqCst);
            false
        }));
        let after_commit = Arc::clone(&committed);
        // The original post-commit capture is executed by Core before asking
        // Native to project its result. Kill only this already observed child.
        db.authorizer(Some(move |context: AuthContext<'_>| {
            if after_commit.load(Ordering::SeqCst)
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "bibliographic_items",
                        ..
                    }
                )
                && !once.swap(true, Ordering::SeqCst)
            {
                terminate(pid);
            }
            Authorization::Allow
        }));
    }
    let error = save(&f).await.unwrap_err();
    assert!(committed.load(Ordering::SeqCst));
    assert!(terminated.load(Ordering::SeqCst));
    assert!(matches!(
        error.code.as_str(),
        "plugin_host_input_failed" | "plugin_host_read_failed" | "plugin_host_exited"
    ));
    let Some(SaveOutcome::Committed { item_id }) = &error.save_outcome else {
        panic!("expected committed result loss: {error:?}");
    };
    assert_eq!(error.instance_id.as_deref(), Some(instance.as_str()));
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert!(error.request_id.is_some());
    assert!(error.cleanup_error.is_none(), "{error:?}");
    {
        let db = f.store.connection().unwrap();
        assert!(db.is_autocommit());
        db.commit_hook(None::<fn() -> bool>);
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        let title: String = db
            .query_row(
                "select title from bibliographic_items where id=?1",
                [item_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, draft().title);
    }
    assert_eq!(counts(&f).0, before.0 + 1);
    let after = counts(&f);
    assert!(f.service.host.lock().await.is_none());
    let rows = f
        .service
        .catalog(
            f.store.shared_agent_connection(),
            f.key.clone(),
            CatalogRequest::Get {
                id: item_id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, *item_id);
    assert_ne!(identity(&f).await.1, instance);
    assert_eq!(counts(&f), after);
    let f = reopen_actual_fixture(f, false).await;
    let reopened = f
        .service
        .catalog(
            f.store.shared_agent_connection(),
            f.key.clone(),
            CatalogRequest::Get {
                id: item_id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reopened, rows);
    assert_eq!(counts(&f), after);
    evidence_matrix::close(f).await;
}
