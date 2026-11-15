//! A real model-settings SQL read keeps its owner after its async waiter leaves.
use super::*;
use crate::agent::repository::WorkerRetention;
use rusqlite::functions::FunctionFlags;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::oneshot;

struct Lease {
    dropped: Arc<AtomicBool>,
    exited: Option<oneshot::Sender<()>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
        let _ = self.exited.take().unwrap().send(());
    }
}
struct Gate {
    entered: oneshot::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
}
impl Gate {
    fn install(f: &Fixture) -> Self {
        let (entered, wait) = oneshot::channel();
        let entered = Mutex::new(Some(entered));
        let (release, released) = mpsc::channel();
        let db = f.store.connection().unwrap();
        db.create_scalar_function(
            "pause_native_model_settings",
            1,
            FunctionFlags::SQLITE_UTF8,
            move |args| {
                if let Some(entered) = entered.lock().unwrap().take() {
                    let _ = entered.send(());
                    released
                        .recv_timeout(Duration::from_secs(20))
                        .map_err(|_| {
                            rusqlite::Error::UserFunctionError(Box::new(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "synthetic model settings gate",
                            )))
                        })?;
                }
                args.get::<i64>(0)
            },
        )
        .unwrap();
        // Equivalent synthetic rows, with an observable read boundary. No
        // preview ModelFactory or credential/provider transport is substituted.
        db.execute_batch("alter table llm_provider_configs rename to synthetic_model_providers;
            create view llm_provider_configs as select provider_id,pause_native_model_settings(enabled) as enabled,base_url,timeout_ms,updated_at from synthetic_model_providers;").unwrap();
        Self {
            entered: wait,
            release: Some(release),
        }
    }
    fn release(mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}
struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn canceled_model_read(replay: bool) {
    let f = managed_fixture(true);
    let operation = Uuid::new_v4();
    if replay {
        WritingSubmissionService::from_app_store(&f.store)
            .prepare_preview(&f.models, request(&f, operation), None)
            .await
            .unwrap();
    }
    let mut gate = Gate::install(&f);
    let (exited, exit) = oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let mut service = WritingSubmissionService::from_app_store(&f.store);
    service.worker_retention = WorkerRetention::new(Lease {
        dropped: dropped.clone(),
        exited: Some(exited),
    });
    let models = f.models.clone();
    let input = request(&f, operation);
    let waiter = tokio::spawn(async move { service.prepare(&models, input).await });
    let _abort = AbortOnDrop(waiter.abort_handle());
    tokio::time::timeout(Duration::from_secs(10), &mut gate.entered)
        .await
        .unwrap()
        .unwrap();
    waiter.abort();
    assert!(matches!(waiter.await,Err(error) if error.is_cancelled()));
    assert!(
        !dropped.load(Ordering::Acquire),
        "the real SQL worker still owns its lease"
    );
    gate.release();
    tokio::time::timeout(Duration::from_secs(10), exit)
        .await
        .unwrap()
        .unwrap();
    assert!(dropped.load(Ordering::Acquire));
    let db = f.store.connection().unwrap();
    for query in [
        "select count(*) from agent_submissions where request_id=?1",
        "select count(*) from data_sql_operation_receipts where operation_id=?1",
    ] {
        assert_eq!(
            db.query_row(query, [operation.to_string()], |row| row.get::<_, u64>(0))
                .unwrap(),
            u64::from(replay)
        );
    }
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn canceled_fresh_prepare_retains_real_model_settings_worker_and_never_starts_materialization()
 {
    canceled_model_read(false).await;
}
#[tokio::test]
async fn canceled_committed_replay_retains_real_model_settings_worker_without_another_preparation()
{
    canceled_model_read(true).await;
}
