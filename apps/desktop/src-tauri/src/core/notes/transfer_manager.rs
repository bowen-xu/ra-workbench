use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::error::NotesError;

#[derive(Clone, Default)]
pub(crate) struct NoteTransferManager {
    is_running: Arc<AtomicBool>,
    is_cancelled: Arc<AtomicBool>,
}

impl NoteTransferManager {
    pub(crate) fn begin(&self) -> Result<(), NotesError> {
        self.is_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| NotesError::TransferBusy)?;
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

    pub(crate) fn check_cancelled(&self) -> Result<(), NotesError> {
        if self.is_cancelled.load(Ordering::Acquire) {
            Err(NotesError::OperationCancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_one_cancellable_transfer() {
        let manager = NoteTransferManager::default();
        manager.begin().expect("first transfer");
        assert!(matches!(manager.begin(), Err(NotesError::TransferBusy)));
        assert!(manager.cancel());
        assert!(matches!(
            manager.check_cancelled(),
            Err(NotesError::OperationCancelled)
        ));
        manager.finish();
        assert!(!manager.cancel());
    }
}
