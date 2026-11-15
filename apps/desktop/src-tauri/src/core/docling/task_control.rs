//! Request-local progress and cooperative cancellation; shared conversions own their own control.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParseProgress {
    pub stage: String,
    pub completed_pages: usize,
    pub total_pages: usize,
}
impl ParseProgress {
    pub fn is_valid(&self) -> bool {
        ["queued", "parsing", "exporting", "cancelling"].contains(&self.stage.as_str())
            && self.completed_pages <= self.total_pages
    }
}
pub(crate) struct ParseControl {
    pub id: String,
    cancelled: AtomicBool,
    wake: Notify,
    progress: Mutex<ParseProgress>,
    source: Mutex<Option<Weak<ParseControl>>>,
}
impl ParseControl {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            cancelled: AtomicBool::new(false),
            wake: Notify::new(),
            progress: Mutex::new(ParseProgress {
                stage: "preparing".into(),
                ..Default::default()
            }),
            source: Mutex::new(None),
        })
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        loop {
            let wake = self.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            wake.await;
        }
    }
    pub fn follow(&self, source: &Arc<Self>) {
        *self.source.lock().unwrap() = Some(Arc::downgrade(source));
    }
    pub fn stage(&self, stage: &str) {
        self.progress.lock().unwrap().stage = stage.into();
    }
    pub fn update(&self, progress: ParseProgress) {
        // Only phase labels defined by the daemon enter the task UI.
        if !progress.is_valid() {
            return;
        }
        let mut current = self.progress.lock().unwrap();
        current.completed_pages = current.completed_pages.max(progress.completed_pages);
        current.total_pages = current.total_pages.max(progress.total_pages);
        current.stage = progress.stage;
    }
    pub fn snapshot(&self) -> ParseProgress {
        let source = self.source.lock().unwrap().as_ref().and_then(Weak::upgrade);
        if let Some(source) = source {
            return source.snapshot();
        }
        self.progress.lock().unwrap().clone()
    }
}
