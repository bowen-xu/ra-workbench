use super::*;
use rusqlite::hooks::Action;
use std::{
    future::Future,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
};
const WAIT: Duration = Duration::from_secs(8);

async fn entered(receiver: mpsc::Receiver<()>) -> bool {
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || receiver.recv_timeout(WAIT)),
    )
    .await
    .is_ok_and(|join| join.is_ok_and(|result| result.is_ok()))
}
#[tokio::test]
async fn aborted_caller_and_shutdown_retain_final_sql_metadata_and_notification() {
    let (notify_entered, notify_rx) = mpsc::channel();
    let (notify_release, notify_release_rx) = mpsc::channel();
    let notify_release_rx = StdMutex::new(notify_release_rx);
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&notifications);
    let events = RuntimeEvents::new(move |name, _| {
        if name == "research-assistant:structured-resource-changed" {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = notify_entered.send(());
            notify_release_rx
                .lock()
                .map_err(|_| "synthetic notify lock")?
                .recv_timeout(WAIT)
                .map_err(|_| "synthetic notify release")?;
        }
        Ok(())
    });
    let values = [portrait(0), portrait(1)];
    let fixture = Fixture::new(&values, false, events);
    let (sql_entered, sql_rx) = mpsc::channel();
    let (sql_release, sql_release_rx) = mpsc::channel();
    let commits = Arc::new(AtomicUsize::new(0));
    {
        let commits = Arc::clone(&commits);
        fixture.store.connection().unwrap().update_hook(Some(
            move |action, database: &str, table: &str, _rowid| {
                if action == Action::SQLITE_INSERT
                    && database == "main"
                    && table == "hbs_commit_receipts"
                {
                    commits.fetch_add(1, Ordering::SeqCst);
                    let _ = sql_entered.send(());
                    let _ = sql_release_rx.recv_timeout(WAIT);
                }
            },
        ));
    }
    let service = Arc::clone(&fixture.service);
    let store = fixture.store.clone();
    let id = fixture.id.clone();
    let caller = tokio::spawn(async move {
        service
            .structured_hbs(
                store,
                id,
                HbsOperation::Commit(crate::hbs_storage::Commit {
                    id: "core-hbs-cancel:1".into(),
                    expected_version: 0,
                    saved: values[0].clone(),
                    baseline: values[1].clone(),
                }),
            )
            .await
    });
    let reached = entered(sql_rx).await;
    if !reached {
        let _ = sql_release.send(());
        let _ = notify_release.send(());
        caller.abort();
        let _ = tokio::time::timeout(WAIT, fixture.service.shutdown()).await;
        panic!("original final HBS receipt SQL was not reached");
    }
    caller.abort();
    let cancelled = caller.await.is_err_and(|cause| cause.is_cancelled());
    let held_during_sql = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let waits_for_sql =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let _ = sql_release.send(());
    let notified = entered(notify_rx).await;
    let waits_for_notify =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let (revision, receipts) = {
        let db = fixture.store.connection().unwrap();
        (
            crate::structured_resources::get(&db, &fixture.id)
                .unwrap()
                .revision,
            db.query_row(
                "select count(*) from hbs_commit_receipts where request_id='core-hbs-cancel:1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        )
    };
    let _ = notify_release.send(());
    let drained = tokio::time::timeout(WAIT, shutdown).await;
    assert!(cancelled && held_during_sql && waits_for_sql);
    assert!(
        notified && waits_for_notify,
        "owner released before original metadata/notification settled"
    );
    assert_eq!((revision, receipts), (1, 1));
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    drained.unwrap().unwrap();
    assert!(fixture.service.host.lock().await.is_none());
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        2
    );
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
    // The original store/pair still contains that one commit after the caller
    // vanished; no retry or replacement request is used to obtain this result.
    let raw = rusqlite::Connection::open(fixture._data.path().join("research-assistant.sqlite3"))
        .unwrap();
    assert_eq!(
        raw.query_row(
            "select count(*) from hbs_commit_receipts where request_id='core-hbs-cancel:1'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
