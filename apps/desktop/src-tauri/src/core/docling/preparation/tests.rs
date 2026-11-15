use super::{test_support::*, *};
use std::sync::atomic::Ordering;

fn fixture() -> (
    tempfile::TempDir,
    AppStore,
    Arc<Parser>,
    Arc<DoclingPreparation>,
) {
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Fixture','now','now')",[]).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    attachment(&store, "paper", "pdf", &"a".repeat(64));
    let parser = Parser::new();
    let preparation = parser.install(&store);
    (root, store, parser, preparation)
}

#[tokio::test]
async fn concurrent_reading_and_chat_share_one_parse_then_reuse_the_saved_version() {
    let (_root, store, parser, preparation) = fixture();
    let a = preparation.ensure(&store, "paper", "pdf");
    let b = preparation.prepare(&store, "paper", "pdf", false);
    let release = async {
        parser.wait_started().await;
        tokio::task::yield_now().await;
        parser.release.add_permits(1);
    };
    let (a, b, ()) = tokio::join!(a, b, release);
    let a = a.unwrap();
    assert_eq!(a.summary.id, b.unwrap().summary.id);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        preparation
            .ensure(&store, "paper", "pdf")
            .await
            .unwrap()
            .summary
            .id,
        a.summary.id
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
    assert!(
        preparation
            .flights
            .lock()
            .unwrap()
            .values()
            .all(|flight| flight.strong_count() == 0)
    );
}

#[tokio::test]
async fn cancelling_one_waiter_does_not_restart_the_other_waiters_conversion() {
    let (_root, store, parser, preparation) = fixture();
    let first = {
        let store = store.clone();
        let preparation = preparation.clone();
        tokio::spawn(async move { preparation.ensure(&store, "paper", "pdf").await })
    };
    parser.wait_started().await;
    let mut second = Box::pin(preparation.ensure(&store, "paper", "pdf"));
    tokio::select! {
        biased;
        _ = &mut second => panic!("conversion must still be waiting"),
        _ = tokio::task::yield_now() => {}
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    parser.release.add_permits(1);
    assert!(second.await.is_ok());
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_parse_is_replaced_and_explicit_reparse_remains_available() {
    let (_root, store, parser, preparation) = fixture();
    parser.release.add_permits(3);
    let first = preparation.ensure(&store, "paper", "pdf").await.unwrap();
    let hash = "b".repeat(64);
    attachment(&store, "paper", "other", &hash);
    store
        .connection()
        .unwrap()
        .execute(
            "update attachments set content_sha256=?1 where id='pdf'",
            [&hash],
        )
        .unwrap();
    let second = preparation.ensure(&store, "paper", "pdf").await.unwrap();
    assert_eq!(second.summary.content_sha256, hash);
    assert_ne!(first.summary.id, second.summary.id);
    let forced = preparation
        .prepare(&store, "paper", "pdf", true)
        .await
        .unwrap();
    assert_ne!(second.summary.id, forced.summary.id);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn simultaneous_failures_are_shared_but_a_later_attempt_can_retry() {
    let (_root, store, parser, preparation) = fixture();
    parser.fail.store(true, Ordering::SeqCst);
    let release = async {
        parser.wait_started().await;
        tokio::task::yield_now().await;
        parser.release.add_permits(1);
    };
    let (a, b, ()) = tokio::join!(
        preparation.ensure(&store, "paper", "pdf"),
        preparation.ensure(&store, "paper", "pdf"),
        release
    );
    assert!(Arc::ptr_eq(&a.unwrap_err(), &b.unwrap_err()));
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
    parser.fail.store(false, Ordering::SeqCst);
    parser.release.add_permits(1);
    assert!(preparation.ensure(&store, "paper", "pdf").await.is_ok());
    assert_eq!(parser.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ownership_trash_and_version_changes_are_checked_before_and_after_conversion() {
    let (_root, store, parser, preparation) = fixture();
    assert!(
        preparation
            .ensure(&store, "unrelated", "pdf")
            .await
            .is_err()
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 0);
    let modify = async {
        parser.wait_started().await;
        let hash = "b".repeat(64);
        attachment(&store, "paper", "other", &hash);
        store
            .connection()
            .unwrap()
            .execute(
                "update attachments set content_sha256=?1 where id='pdf'",
                [&hash],
            )
            .unwrap();
        parser.release.add_permits(1);
    };
    let (result, ()) = tokio::join!(preparation.ensure(&store, "paper", "pdf"), modify);
    assert!(matches!(
        result.unwrap_err().as_ref(),
        DoclingError::Attachment(_)
    ));
    store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set trashed_at='now' where id='paper'",
            [],
        )
        .unwrap();
    assert!(preparation.ensure(&store, "paper", "pdf").await.is_err());
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn partial_success_for_the_current_hash_is_also_reused() {
    let (_root, store, parser, preparation) = fixture();
    parser.release.add_permits(1);
    let run = preparation.ensure(&store, "paper", "pdf").await.unwrap();
    store
        .connection()
        .unwrap()
        .execute(
            "update docling_parse_runs set parse_status='partial_success' where id=?1",
            [&run.summary.id],
        )
        .unwrap();
    assert_eq!(
        preparation
            .ensure(&store, "paper", "pdf")
            .await
            .unwrap()
            .summary
            .parse_status,
        "partial_success"
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn model_identity_separates_native_results_from_legacy_and_other_models() {
    let (_root, store, parser, preparation) = fixture();
    parser.release.add_permits(1);
    let legacy = preparation.ensure(&store, "paper", "pdf").await.unwrap();
    let hash = legacy.summary.content_sha256;
    assert!(
        cached(&store, "paper", "pdf", &hash, Some("native-model-v1"))
            .unwrap()
            .is_none()
    );
    let db = store.connection().unwrap();
    db.execute(
        "update docling_parse_runs set raw_response=?1 where id=?2",
        params![
            serde_json::json!({"runtime":{"identity":"native-model-v1"}}).to_string(),
            legacy.summary.id
        ],
    )
    .unwrap();
    drop(db);
    assert!(
        cached(&store, "paper", "pdf", &hash, Some("native-model-v1"))
            .unwrap()
            .is_some()
    );
    assert!(
        cached(&store, "paper", "pdf", &hash, Some("native-model-v2"))
            .unwrap()
            .is_none()
    );
    // An unchanged FP32 model must not reuse output from the old assembler.
    let corrected = format!(
        "native-model-v1:parser={}",
        research_docling_runtime::PARSER_REVISION
    );
    assert!(
        cached(&store, "paper", "pdf", &hash, Some(&corrected))
            .unwrap()
            .is_none()
    );
    store
        .connection()
        .unwrap()
        .execute(
            "update docling_parse_runs set raw_response=?1 where id=?2",
            params![
                serde_json::json!({"runtime":{"identity":corrected}}).to_string(),
                legacy.summary.id
            ],
        )
        .unwrap();
    assert!(
        cached(&store, "paper", "pdf", &hash, Some(&corrected))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn manual_cancel_detaches_only_its_waiter_and_preserves_shared_progress() {
    let (_root, store, parser, preparation) = fixture();
    let observer = super::super::task_control::ParseControl::new();
    let a = preparation.prepare_controlled(&store, "paper", "pdf", true, Some(observer.clone()));
    let b = preparation.ensure(&store, "paper", "pdf");
    let release = async {
        parser.wait_started().await;
        tokio::task::yield_now().await;
        observer.cancel();
        tokio::task::yield_now().await;
        parser.release.add_permits(1);
    };
    let (a, b, ()) = tokio::join!(a, b, release);
    assert!(matches!(a, Err(ref error) if matches!(error.as_ref(), DoclingError::Cancelled)));
    assert!(b.is_ok());
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancel_before_preparation_never_starts_a_conversion() {
    let (_root, store, parser, preparation) = fixture();
    let observer = super::super::task_control::ParseControl::new();
    observer.cancel();
    let result = preparation
        .prepare_controlled(&store, "paper", "pdf", true, Some(observer))
        .await;
    assert!(matches!(result, Err(ref error) if matches!(error.as_ref(), DoclingError::Cancelled)));
    assert_eq!(parser.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn last_manual_waiter_confirms_cancellation_only_after_the_processing_boundary() {
    struct Controlled {
        started: tokio::sync::Semaphore,
        saw_cancel: tokio::sync::Semaphore,
        boundary: tokio::sync::Semaphore,
    }
    impl AttachmentParser for Controlled {
        fn parse<'a>(&'a self, _: &'a AppStore, _: String, _: String) -> ParseFuture<'a> {
            unreachable!("controlled dispatch required")
        }
        fn parse_controlled<'a>(
            &'a self,
            _: &'a AppStore,
            _: String,
            _: String,
            control: Arc<super::super::task_control::ParseControl>,
        ) -> ParseFuture<'a> {
            Box::pin(async move {
                control.update(super::super::task_control::ParseProgress {
                    stage: "parsing".into(),
                    completed_pages: 1,
                    total_pages: 12,
                });
                self.started.add_permits(1);
                control.cancelled().await;
                self.saw_cancel.add_permits(1);
                self.boundary.acquire().await.unwrap().forget();
                Err(DoclingError::Cancelled)
            })
        }
    }
    let (_root, store, _parser, _preparation) = fixture();
    let parser = Arc::new(Controlled {
        started: tokio::sync::Semaphore::new(0),
        saw_cancel: tokio::sync::Semaphore::new(0),
        boundary: tokio::sync::Semaphore::new(0),
    });
    let preparation = Arc::new(DoclingPreparation::with_parser(parser.clone()));
    let observer = super::super::task_control::ParseControl::new();
    let observed = observer.clone();
    let task_store = store.clone();
    let mut task = tokio::spawn(async move {
        preparation
            .prepare_controlled(&task_store, "paper", "pdf", true, Some(observed))
            .await
    });
    parser.started.acquire().await.unwrap().forget();
    assert_eq!(observer.snapshot().completed_pages, 1);
    observer.cancel();
    parser.saw_cancel.acquire().await.unwrap().forget();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut task)
            .await
            .is_err()
    );
    parser.boundary.add_permits(1);
    assert!(
        matches!(task.await.unwrap(), Err(error) if matches!(error.as_ref(), DoclingError::Cancelled))
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("select count(*) from docling_parse_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
