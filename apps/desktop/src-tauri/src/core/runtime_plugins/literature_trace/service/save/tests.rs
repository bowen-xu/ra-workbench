#![cfg(unix)]
use super::*;
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn real_project_exit_after_commit_keeps_the_committed_outcome_and_never_replays() {
    let fixture = Fixture::new(None).await;
    assert!(fixture.db.logged());
    assert!(fixture.data.path().join("store.sqlite3").is_file());
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let draft = fixture.draft.clone();
    let client = tokio::spawn(async move { service.save_catalog(database, key, draft).await });
    fixture.prepare_seen().await;

    // This is the production Database::lock, including its materialize/admission
    // owner. It must be usable while the real child awaits the prepare release.
    let database = Arc::clone(&fixture.db);
    let mut access = tokio::task::spawn_blocking(move || {
        database
            .lock()
            .unwrap()
            .query_row("select count(*) from bibliographic_items", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    });
    let accessible = timeout(Duration::from_secs(2), &mut access).await;
    fixture.release_prepare();
    assert_eq!(
        accessible
            .expect("DB/admission guard crossed native await")
            .unwrap(),
        0
    );

    let error = timeout(WAIT, client).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_exited");
    assert_eq!(error.exit_code, Some(41));
    let Some(SaveOutcome::Committed { item_id }) = &error.save_outcome else {
        panic!("expected known committed output failure: {error:?}");
    };
    uuid::Uuid::parse_str(item_id).unwrap();
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    assert!(fixture.service.host.lock().await.is_none());
    assert!(!fixture.directory.exists() && !fixture.executable.exists());
    {
        let db = fixture.db.lock().unwrap();
        let saved: (String, String) = db
            .query_row("select id,title from bibliographic_items", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(saved, (item_id.clone(), "Committed synthetic title".into()));
        for sql in [
            "select count(*) from bibliographic_items",
            "select count(*) from creators",
            "select count(*) from literature_search",
        ] {
            assert_eq!(
                db.query_row(sql, [], |row| row.get::<_, u64>(0)).unwrap(),
                1
            );
        }
    }
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}

#[tokio::test]
async fn aborted_client_does_not_abort_t1_and_shutdown_waits_for_one_reopenable_commit() {
    use crate::data_interface::Database;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::sync::{atomic::AtomicUsize, mpsc};

    let fixture = Fixture::new(Some("lifecycle-synthetic-id")).await;
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_catalog_save_barrier",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                entered_tx
                    .send(())
                    .map_err(|cause| rusqlite::Error::UserFunctionError(Box::new(cause)))?;
                release_rx
                    .recv()
                    .map_err(|cause| rusqlite::Error::UserFunctionError(Box::new(cause)))?;
                Ok(0_i64)
            },
        )
        .unwrap();
        // Connection-local instrumentation; deployed schema and package SQL stay intact.
        db.execute_batch(
            "create temp trigger test_catalog_save_pause before insert on bibliographic_items
            begin select test_catalog_save_barrier(); end;",
        )
        .unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let draft = fixture.draft.clone();
    let client = tokio::spawn(async move { service.save_catalog(database, key, draft).await });
    fixture.prepare_seen().await;
    fixture.release_prepare();
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();

    // The SQL trigger proves T1 is active. Aborting only the client's wait must
    // leave the production Core task, and its existing host mutex, alive.
    client.abort();
    let cancelled = client.await;
    let retained_host_owner = fixture.service.host.try_lock().is_err();
    let service = Arc::clone(&fixture.service);
    let mut shutdown = tokio::spawn(async move { service.shutdown().await });
    timeout(WAIT, async {
        while !fixture.service.stopping.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let probe = timeout(Duration::from_millis(100), &mut shutdown).await;
    release_tx.send(()).unwrap();
    let (waited_for_t1, result) = match probe {
        Err(_) => (true, timeout(WAIT, shutdown).await.unwrap()),
        Ok(result) => (false, result),
    };
    assert!(cancelled.is_err_and(|cause| cause.is_cancelled()));
    assert!(
        retained_host_owner,
        "client abort released the Core-owned save"
    );
    assert!(
        waited_for_t1,
        "shutdown completed before the active SQL was released"
    );
    result.unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(fixture.service.host.lock().await.is_none());
    assert!(!fixture.directory.exists() && !fixture.executable.exists());

    let Fixture { data, db, .. } = fixture;
    db.shutdown().unwrap();
    drop(db);
    let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened = Database::open(sql, &data.path().join("data-log")).unwrap();
    {
        let db = reopened.lock().unwrap();
        let saved: (String, String) = db
            .query_row("select id,title from bibliographic_items", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(
            saved,
            (
                "lifecycle-synthetic-id".into(),
                "Committed synthetic title".into()
            )
        );
        assert_eq!(
            db.query_row("select count(*) from bibliographic_items", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("select count(*) from creators", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
    reopened.shutdown().unwrap();
}
