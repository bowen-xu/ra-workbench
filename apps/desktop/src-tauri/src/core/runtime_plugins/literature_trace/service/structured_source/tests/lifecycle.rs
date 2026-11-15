use super::*;
use rusqlite::hooks::Action;
use std::{future::Future, sync::mpsc, task::Poll};

#[tokio::test]
async fn shutdown_during_actual_native_exchange_retains_that_frame_request_identity() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    fixture.configure(&[prepared.build], Some(0), false);
    let caller = fixture.spawn();
    let entered = fixture.entered(0).await;
    let actual_request = fixture
        .lifecycle()
        .lines()
        .find_map(|line| line.strip_prefix("request ").map(str::to_owned));
    let drained = tokio::time::timeout(WAIT, fixture.service.shutdown()).await;
    // Also unblock the finite shell gate if a failed shutdown needs cleanup.
    fixture.release();
    let result = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    drained.unwrap().unwrap();
    assert!(entered);
    let actual_request =
        actual_request.expect("actual first exchange request was logged before its gate");
    assert!(uuid::Uuid::parse_str(&actual_request).is_ok());
    let output = result
        .unwrap_err()
        .into_output(&fixture.record, &fixture.call);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert!(output.is_error);
    assert_eq!(value["requestId"], fixture.record.request_id.to_string());
    assert_eq!(value["nativeFailure"]["nativeRequestId"], actual_request);
    assert_eq!(value["nativeFailure"]["packageKey"], fixture.key);
    assert_eq!(fixture.counts(), (0, 0));
    assert!(fixture.service.host.lock().await.is_none());
}

#[tokio::test]
async fn cancelled_waiter_keeps_owner_while_core_note_write_and_shutdown_drain_final_proof() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    fixture.configure(&[prepared.build, prepared.project], Some(0), false);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    fixture.store.connection().unwrap().update_hook(Some(
        move |action, database: &str, table: &str, _rowid| {
            if action == Action::SQLITE_INSERT
                && database == "main"
                && table == "agent_text_read_scopes"
            {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(WAIT);
            }
        },
    ));
    let caller = fixture.spawn();
    let native_entered = fixture.entered(0).await;
    caller.abort();
    let cancelled = caller.await.is_err_and(|error| error.is_cancelled());
    // The Native wait retains no DB/content guard. Exercise the real Core Note
    // creation transaction, not a private test-table mutation.
    let store = fixture.store.clone();
    let note = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            let id = crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Independent Core note".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                None,
            )
            .unwrap();
            tx.commit().unwrap();
            id
        }),
    )
    .await;
    fixture.release();
    let proof_entered = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await
    .is_ok_and(|join| join.is_ok_and(|received| received.is_ok()));
    let held = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let _ = release_tx.send(());
    tokio::time::timeout(WAIT, shutdown).await.unwrap().unwrap();
    assert!(native_entered && cancelled && proof_entered && held && pending);
    let note = note.unwrap().unwrap();
    let db = fixture.store.connection().unwrap();
    assert_eq!(fixture::counts(&db), (1, 1));
    assert_eq!(
        db.query_row("select title from note_pages where id=?1", [note], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "Independent Core note"
    );
    drop(db);
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
    assert!(fixture.service.host.lock().await.is_none());
}
