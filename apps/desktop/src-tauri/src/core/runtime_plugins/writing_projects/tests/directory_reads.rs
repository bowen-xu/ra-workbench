use super::*;
use super::{
    directories::{directory, rename, source},
    files::{seed, store},
};
use crate::runtime_plugins::literature_catalog::save::SaveOutcome;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_rechecks_session_after_native_without_holding_database() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "chapters/a.tex", "A");
    let captured = Arc::new(tokio::sync::Notify::new());
    let (release, waiting) = std::sync::mpsc::channel();
    {
        let db = fixture.database.lock().unwrap();
        db.execute("insert into writing_sessions values(?1,'[\"chapters/a.tex\"]','chapters/a.tex','{}','{}','old')",[&fixture.active]).unwrap();
        let signal = Arc::clone(&captured);
        let seen = AtomicBool::new(false);
        db.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "writing_sessions",
                    ..
                }
            ) && !seen.swap(true, Ordering::SeqCst)
            {
                signal.notify_one();
                if waiting.recv_timeout(Duration::from_secs(90)).is_err() {
                    return Authorization::Deny;
                }
            }
            Authorization::Allow
        }));
    }
    let operation_store = store.clone();
    let active = fixture.active.clone();
    let operation_fixture = Arc::new(fixture);
    let borrowed = Arc::clone(&operation_fixture);
    let changing = tokio::spawn(async move {
        directory(
            &borrowed,
            &operation_store,
            rename(&active, "chapters", "renamed"),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(90), captured.notified())
        .await
        .unwrap();
    let database = Arc::clone(&operation_fixture.database);
    let active = operation_fixture.active.clone();
    let entered = Arc::new(tokio::sync::Notify::new());
    let entering = Arc::clone(&entered);
    let writer = tokio::task::spawn_blocking(move || {
        entering.notify_one();
        database.lock().unwrap().execute("update writing_sessions set layout_state_json='{\"concurrent\":true}' where manuscript_id=?1",[active]).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(90), entered.notified())
        .await
        .unwrap();
    release.send(()).unwrap();
    writer.await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(90), changing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.session"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        source(&store, &operation_fixture.active)
            .join("chapters/a.tex")
            .exists()
    );
    assert!(
        !source(&store, &operation_fixture.active)
            .join("renamed")
            .exists()
    );
    {
        let db = operation_fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_files where relative_path='chapters/a.tex'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "select layout_state_json from writing_sessions where manuscript_id=?1",
                [&operation_fixture.active],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            r#"{"concurrent":true}"#
        );
        assert!(db.is_autocommit());
    }
    operation_fixture.service.shutdown().await.unwrap();
}
