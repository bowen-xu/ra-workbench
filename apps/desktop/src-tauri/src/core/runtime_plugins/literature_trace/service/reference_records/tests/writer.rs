//! One original INSERT prepare is held; Drop always releases a blocked worker.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

pub(super) struct WriterGate {
    entered: Arc<Notify>,
    release: Arc<(Mutex<bool>, Condvar)>,
    pub attempts: Arc<AtomicUsize>,
}

impl WriterGate {
    pub fn new(db: &crate::data_interface::SharedDatabase) -> Self {
        let gate = Self {
            entered: Arc::new(Notify::new()),
            release: Arc::new((Mutex::new(false), Condvar::new())),
            attempts: Arc::new(AtomicUsize::new(0)),
        };
        let entered = Arc::clone(&gate.entered);
        let release = Arc::clone(&gate.release);
        let attempts = Arc::clone(&gate.attempts);
        db.lock()
            .unwrap()
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Insert {
                        table_name: "reference_records"
                    }
                ) && attempts.fetch_add(1, Ordering::SeqCst) == 0
                {
                    entered.notify_one();
                    let (lock, condition) = release.as_ref();
                    let released = lock.lock().unwrap();
                    drop(condition.wait_while(released, |value| !*value).unwrap());
                }
                Authorization::Allow
            }));
        gate
    }

    pub async fn entered(&self) {
        timeout(WAIT, self.entered.notified()).await.unwrap();
    }

    pub fn release(&self) {
        let (lock, condition) = self.release.as_ref();
        *lock.lock().unwrap() = true;
        condition.notify_all();
    }
}

impl Drop for WriterGate {
    fn drop(&mut self) {
        self.release();
    }
}
