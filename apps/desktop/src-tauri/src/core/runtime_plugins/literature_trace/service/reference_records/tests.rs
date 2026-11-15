#![cfg(unix)]
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::{future::Future, sync::atomic::AtomicUsize, task::Poll};
use tokio::time::timeout;

mod fixture;
#[path = "tests/writer.rs"]
mod writer;
use fixture::{Fixture, Mode, snapshot};

const WAIT: Duration = Duration::from_secs(8);

fn failure(result: NativeResult<ReferenceResult>) -> Box<NativeHostError> {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected reference failure"),
    }
}

fn identity(error: &NativeHostError, fixture: &Fixture, dispatched: bool) {
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert_eq!(error.plugin_id, "literature_library");
    if dispatched {
        let request = fixture.request();
        assert_eq!(request.package_key, fixture.key);
        assert_eq!(request.instance_id, fixture.initial_identity.0);
        assert_eq!(
            error.instance_id.as_deref(),
            Some(request.instance_id.as_str())
        );
        assert_eq!(
            error.request_id.as_deref(),
            Some(request.request_id.as_str())
        );
        assert_ne!(request.request_id, fixture.initial_identity.1);
    } else {
        assert!(error.instance_id.is_none() && error.request_id.is_none());
    }
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    let wire = serde_json::to_string(error).unwrap();
    assert!(!wire.contains("Preserved synthetic duplicate Core body"));
    assert!(!wire.contains("Core Note saved while reference preparation waits"));
    assert!(!wire.contains("Synthetic reference body"));
}

#[tokio::test]
async fn native_wait_allows_core_note_and_abort_cannot_cancel_the_accepted_writer_or_shutdown() {
    let fixture = Fixture::new(Mode::Held).await;
    assert!(fixture.data.db.logged());
    let original = fixture.data.note();
    let client = fixture.start();
    fixture.entered().await;
    assert!(fixture.service.host.try_lock().is_err());
    let db = Arc::clone(&fixture.data.db);
    let saved = timeout(
        WAIT,
        tokio::task::spawn_blocking(move || fixture::save_note(&db)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(saved, original);
    assert_eq!(fixture.data.note(), saved);

    let writer = writer::WriterGate::new(&fixture.data.db);
    fixture.release();
    writer.entered().await;
    // The actual original INSERT is now preparing inside the accepted blocking
    // writer, beyond self.available(). Stopping during preparation proves less.
    client.abort();
    assert!(client.await.is_err_and(|cause| cause.is_cancelled()));
    assert!(fixture.service.host.try_lock().is_err());
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    writer.release();
    assert!(
        pending && stopping,
        "shutdown did not wait for the accepted SQL owner"
    );
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    assert_eq!(writer.attempts.load(Ordering::SeqCst), 1);
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(fixture.steps(), ["initialize", "prepare", "shutdown"]);
    fixture.cleaned().await;
    let state = snapshot(&fixture.data.db);
    let records = state["records"].as_array().unwrap();
    let revisions = state["revisions"].as_array().unwrap();
    assert_eq!((records.len(), revisions.len()), (1, 1));
    let id = records[0][0].as_str().unwrap();
    uuid::Uuid::parse_str(id).unwrap();
    assert_eq!(records[0][1], "survivor");
    assert_eq!(records[0][2], "reference-service-anchor");
    assert_eq!(records[0][3], "Synthetic reference body");
    assert_eq!(records[0][13], 1);
    assert_eq!(revisions[0][1], id);
    assert_eq!(revisions[0][2], 1);
    assert_eq!(revisions[0][4], "human");
    assert_eq!(revisions[0][5], "manual_reference_capture");
    let raw: research_workbench_native_sdk::reference_records::ReferenceRecordDraft =
        serde_json::from_str(revisions[0][3].as_str().unwrap()).unwrap();
    assert_eq!(raw, fixture.draft);
    assert_eq!(state["note"], saved);
    drop(writer);
    let data = fixture.cold();
    assert_eq!(snapshot(&data.db), state);
    data.db.shutdown().unwrap();
}

#[tokio::test]
async fn wrong_draft_or_catalog_kind_retires_the_real_instance_without_domain_sql() {
    // Two distinct protocol faults; this is not a retry of a failed fixture.
    for mode in [Mode::WrongDraft, Mode::WrongKind] {
        let fixture = Fixture::new(mode).await;
        let before = snapshot(&fixture.data.db);
        let observations = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&observations);
        fixture
            .data
            .db
            .lock()
            .unwrap()
            .authorizer(Some(move |context: AuthContext<'_>| {
                let table = match context.action {
                    AuthAction::Read { table_name, .. }
                    | AuthAction::Insert { table_name }
                    | AuthAction::Update { table_name, .. }
                    | AuthAction::Delete { table_name } => Some(table_name),
                    _ => None,
                };
                if table.is_some_and(|name| {
                    matches!(
                        name,
                        "reference_records"
                            | "reference_record_revisions"
                            | "source_anchors"
                            | "bibliographic_items"
                    )
                }) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                Authorization::Allow
            }));
        let error = failure(timeout(WAIT, fixture.start()).await.unwrap().unwrap());
        assert_eq!(error.code, "plugin_host_response_mismatch");
        identity(&error, &fixture, true);
        assert_eq!(
            error.reference_records.as_ref().unwrap().progress.outcome,
            crate::runtime_plugins::literature_reference_records::ReferenceOutcome::NotStarted
        );
        assert!(
            error
                .reference_records
                .as_ref()
                .unwrap()
                .progress
                .record_id
                .is_none()
        );
        assert_eq!(observations.load(Ordering::SeqCst), 0);
        fixture
            .data
            .db
            .lock()
            .unwrap()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        assert_eq!(fixture.steps(), ["initialize", "prepare"]);
        fixture.cleaned().await;
        assert_eq!(snapshot(&fixture.data.db), before);
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        let data = fixture.cold();
        assert_eq!(snapshot(&data.db), before);
        data.db.shutdown().unwrap();
    }
}

#[tokio::test]
async fn owner_fence_precedes_invalid_draft_and_stopped_service_never_dispatches_again() {
    // The same canned invalid-input reply is reached with an admitted owner;
    // the fenced variant must fail before it can send any prepare request.
    for fenced in [false, true] {
        let fixture = Fixture::new(Mode::Rejected).await;
        let before = snapshot(&fixture.data.db);
        if fenced {
            fixture.data.db.lock().unwrap().require_recovery();
            assert!(fixture.data.db.lock().is_err());
        }
        let error = failure(timeout(WAIT, fixture.start()).await.unwrap().unwrap());
        identity(&error, &fixture, !fenced);
        let progress = &error.reference_records.as_ref().unwrap().progress;
        assert_eq!(
            progress.outcome,
            crate::runtime_plugins::literature_reference_records::ReferenceOutcome::NotStarted
        );
        if fenced {
            assert_eq!(error.code, "storage_error");
            assert_eq!(progress.phase, "owner");
            assert!(matches!(
                &error.reference_records.as_ref().unwrap().problem,
                ReferenceProblem::Owner(_)
            ));
            assert_eq!(fixture.steps(), ["initialize"]);
        } else {
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.message, "参考条目字段无效");
            assert_eq!(progress.phase, "validation");
            assert_eq!(fixture.steps(), ["initialize", "prepare"]);
            assert_eq!(snapshot(&fixture.data.db), before);
        }
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        fixture.cleaned().await;
        let calls = fixture.steps();
        let stopped = failure(timeout(WAIT, fixture.start()).await.unwrap().unwrap());
        identity(&stopped, &fixture, false);
        assert_eq!(stopped.code, "plugin_host_stopping");
        assert_eq!(
            stopped.reference_records.as_ref().unwrap().progress.outcome,
            crate::runtime_plugins::literature_reference_records::ReferenceOutcome::NotStarted
        );
        assert_eq!(fixture.steps(), calls);
        if fenced {
            assert_eq!(fixture.cold_fenced(), before);
        } else {
            let data = fixture.cold();
            assert_eq!(snapshot(&data.db), before);
            data.db.shutdown().unwrap();
        }
    }
}

// The legacy signed increment panics in debug builds. Keep that behavior and
// exercise the real blocking worker's JoinError, rather than injecting one.
#[cfg(debug_assertions)]
#[tokio::test]
async fn writer_panic_retains_actual_prepare_identity_and_fences_without_replaying() {
    let fixture = Fixture::new(Mode::Overflow).await;
    let before = snapshot(&fixture.data.db);
    assert_eq!(before["records"].as_array().unwrap().len(), 1);
    assert_eq!(before["records"][0][13], i64::MAX);
    assert_eq!(before["revisions"].as_array().unwrap().len(), 1);
    let header_reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let observed_headers = Arc::clone(&header_reads);
    let observed_writes = Arc::clone(&writes);
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            match context.action {
                AuthAction::Read {
                    table_name: "reference_records",
                    column_name: "current_revision",
                } => {
                    observed_headers.fetch_add(1, Ordering::SeqCst);
                }
                AuthAction::Insert { table_name }
                | AuthAction::Update { table_name, .. }
                | AuthAction::Delete { table_name }
                    if matches!(
                        table_name,
                        "reference_records" | "reference_record_revisions"
                    ) =>
                {
                    observed_writes.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
            Authorization::Allow
        }));

    let error = failure(timeout(WAIT, fixture.start()).await.unwrap().unwrap());
    assert_eq!(error.code, "storage_error");
    identity(&error, &fixture, true);
    let diagnostic = error.reference_records.as_ref().unwrap();
    assert_eq!(diagnostic.progress.phase, "execution");
    assert_eq!(
        diagnostic.progress.outcome,
        crate::runtime_plugins::literature_reference_records::ReferenceOutcome::Unknown
    );
    assert!(matches!(
        &diagnostic.problem,
        ReferenceProblem::Owner("REFERENCE_WRITER_UNSETTLED")
    ));
    assert!(error.message.contains("禁止自动重试"));
    assert_eq!(header_reads.load(Ordering::SeqCst), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.steps(), ["initialize", "prepare"]);
    assert!(fixture.data.db.lock().is_err());
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.cleaned().await;
    assert_eq!(fixture.steps(), ["initialize", "prepare", "shutdown"]);
    assert_eq!(header_reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cold_fenced(), before);
}
