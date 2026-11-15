use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::{error::ContentError, model::ContentGcStatus};

#[derive(Clone)]
pub(crate) struct ContentGcManager {
    state: Arc<Mutex<String>>,
    cancelled: Arc<AtomicBool>,
}

impl Default for ContentGcManager {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new("idle".to_owned())),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ContentGcManager {
    pub(crate) fn begin(&self) -> Result<(), ContentError> {
        let mut state = self.state.lock().map_err(|_| ContentError::Busy)?;
        if state.as_str() != "idle" {
            return Err(ContentError::Busy);
        }
        self.cancelled.store(false, Ordering::SeqCst);
        *state = "backfill".to_owned();
        Ok(())
    }

    pub(crate) fn set_phase(&self, phase: &str) -> Result<(), ContentError> {
        let mut state = self.state.lock().map_err(|_| ContentError::Busy)?;
        *state = phase.to_owned();
        Ok(())
    }

    pub(crate) fn finish(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = "idle".to_owned();
        }
        self.cancelled.store(false, Ordering::SeqCst);
    }

    pub(crate) fn cancel(&self) -> bool {
        let running = self
            .state
            .lock()
            .map(|state| state.as_str() != "idle")
            .unwrap_or(false);
        if running {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        running
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), ContentError> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(ContentError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn status(&self) -> ContentGcStatus {
        let phase = self
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| "failed".to_owned());
        ContentGcStatus {
            is_cancellable: phase != "idle",
            phase,
        }
    }
}
