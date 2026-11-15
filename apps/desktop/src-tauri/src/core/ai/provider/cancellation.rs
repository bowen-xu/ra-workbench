use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub(crate) struct CancellationSignal {
    is_cancelled: Arc<AtomicBool>,
    notification: Arc<Notify>,
    commit_gate: Arc<Mutex<()>>,
}

pub(crate) struct Cancelled;

impl CancellationSignal {
    pub(crate) fn cancel(&self) {
        let _commit = self
            .commit_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.is_cancelled.store(true, Ordering::Release);
        self.notification.notify_waiters();
    }

    /// Linearize a short durable commit with cancellation. External I/O and
    /// transaction preparation must occur before this synchronous boundary.
    pub(crate) fn commit_if_active<T>(&self, commit: impl FnOnce() -> T) -> Result<T, Cancelled> {
        let _commit = self.commit_gate.lock().map_err(|_| Cancelled)?;
        if self.is_cancelled() {
            return Err(Cancelled);
        }
        Ok(commit())
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        // Register the notification generation before checking the atomic flag:
        // notify_waiters between the check and registration must not be lost.
        let notified = self.notification.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }

    pub(crate) async fn race<F, Output>(&self, operation: F) -> Result<Output, Cancelled>
    where
        F: Future<Output = Output>,
    {
        let mut operation = pin!(operation);
        let mut cancelled = pin!(self.cancelled());
        poll_fn(|context| {
            if self.is_cancelled() || cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(Cancelled));
            }
            operation.as_mut().poll(context).map(Ok)
        })
        .await
    }
}

#[cfg(test)]
mod commit_tests {
    use super::*;
    #[test]
    fn canceled_signal_cannot_claim_a_commit() {
        let signal = CancellationSignal::default();
        signal.cancel();
        assert!(
            signal
                .commit_if_active(|| panic!("must not commit"))
                .is_err()
        );
    }
    #[test]
    fn claimed_commit_finishes_before_cancellation_is_published() {
        let signal = CancellationSignal::default();
        let commit_signal = signal.clone();
        let cancel_signal = signal.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (canceled_tx, canceled_rx) = std::sync::mpsc::channel();
        let commit = std::thread::spawn(move || {
            assert!(
                commit_signal
                    .commit_if_active(|| {
                        entered_tx.send(()).unwrap();
                        release_rx
                            .recv_timeout(std::time::Duration::from_secs(2))
                            .unwrap();
                        assert!(!commit_signal.is_cancelled());
                    })
                    .is_ok()
            );
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let cancel = std::thread::spawn(move || {
            cancel_signal.cancel();
            canceled_tx.send(()).unwrap();
        });
        assert!(matches!(
            canceled_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        release_tx.send(()).unwrap();
        commit.join().unwrap();
        cancel.join().unwrap();
        assert!(signal.is_cancelled());
        canceled_rx.recv().unwrap();
    }
}
