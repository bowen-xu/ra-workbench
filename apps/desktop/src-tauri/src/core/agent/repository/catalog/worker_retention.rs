//! Retain the caller's existing I/O lease in the actual blocking worker.
//! This has no admission, cancellation, task lookup or retry policy of its own.
use std::sync::Arc;
use tokio::task::JoinHandle;

#[derive(Clone, Default)]
pub(crate) struct WorkerRetention {
    _retained: Option<Arc<dyn Send + Sync>>,
}
impl WorkerRetention {
    #[cfg(test)]
    pub(crate) fn new(retained: impl Send + Sync + 'static) -> Self {
        Self {
            _retained: Some(Arc::new(retained)),
        }
    }

    pub(crate) fn spawn<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let retained = self.clone();
        tokio::task::spawn_blocking(move || {
            // A canceled or panicking async parent can drop its JoinHandle and
            // lease while this worker still owns a SQL transaction or receipt.
            // Release only after the real operation returns or unwinds. Unwind
            // is not proof of SQL rollback: existing callers retain Unknown.
            let _retained = retained;
            operation()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use tokio::sync::oneshot;

    struct Lease {
        dropped: Arc<AtomicBool>,
        finished: Option<oneshot::Sender<()>>,
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
            let _ = self.finished.take().unwrap().send(());
        }
    }

    #[tokio::test]
    async fn real_worker_retains_the_existing_lease_after_its_join_handle_is_dropped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let (finished, finished_wait) = oneshot::channel();
        let retention = WorkerRetention::new(Lease {
            dropped: dropped.clone(),
            finished: Some(finished),
        });
        let (entered, started) = oneshot::channel();
        let (release, released) = mpsc::channel();
        let worker = retention.spawn(move || {
            let _ = entered.send(());
            let _ = released.recv();
        });
        drop(retention);
        started.await.unwrap();
        drop(worker);
        assert!(!dropped.load(Ordering::Acquire));
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), finished_wait)
            .await
            .unwrap()
            .unwrap();
        assert!(dropped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn aborted_or_panicking_async_parent_does_not_release_a_running_worker_lease() {
        for panic in [false, true] {
            let dropped = Arc::new(AtomicBool::new(false));
            let (finished, finished_wait) = oneshot::channel();
            let retention = WorkerRetention::new(Lease {
                dropped: dropped.clone(),
                finished: Some(finished),
            });
            let (entered, started) = oneshot::channel();
            let (release, released) = mpsc::channel();
            let (fail, failure) = oneshot::channel();
            let parent = tokio::spawn(async move {
                let _worker = retention.spawn(move || {
                    let _ = entered.send(());
                    let _ = released.recv();
                });
                failure.await.unwrap();
                panic!("synthetic outer async panic");
            });
            started.await.unwrap();
            if panic {
                fail.send(()).unwrap();
                assert!(parent.await.unwrap_err().is_panic());
            } else {
                parent.abort();
                assert!(parent.await.unwrap_err().is_cancelled());
            }
            assert!(!dropped.load(Ordering::Acquire));
            release.send(()).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), finished_wait)
                .await
                .unwrap()
                .unwrap();
            assert!(dropped.load(Ordering::Acquire));
        }
    }
}
