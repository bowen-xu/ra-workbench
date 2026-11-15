use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::ResearchExportError;

#[derive(Clone, Default)]
pub(crate) struct ResearchExportManager {
    is_running: Arc<AtomicBool>,
    is_cancelled: Arc<AtomicBool>,
}

impl ResearchExportManager {
    pub(crate) fn begin(&self) -> Result<(), ResearchExportError> {
        self.is_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ResearchExportError::Busy)?;
        self.is_cancelled.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn finish(&self) {
        self.is_cancelled.store(false, Ordering::Release);
        self.is_running.store(false, Ordering::Release);
    }

    pub(crate) fn cancel(&self) -> bool {
        if !self.is_running.load(Ordering::Acquire) {
            return false;
        }
        self.is_cancelled.store(true, Ordering::Release);
        true
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), ResearchExportError> {
        if self.is_cancelled.load(Ordering::Acquire) {
            Err(ResearchExportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_one_cancellable_export() {
        let manager = ResearchExportManager::default();
        manager.begin().expect("first export");
        assert!(matches!(manager.begin(), Err(ResearchExportError::Busy)));
        assert!(manager.cancel());
        assert!(matches!(
            manager.check_cancelled(),
            Err(ResearchExportError::Cancelled)
        ));
        manager.finish();
        assert!(!manager.cancel());
    }
}
