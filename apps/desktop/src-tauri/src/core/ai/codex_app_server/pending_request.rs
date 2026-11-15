//! The request future owns its registration, including when the future is dropped.
use super::*;

pub(super) struct PendingRequest<'a> {
    pending: &'a StdMutex<Pending>,
    id: u64,
}

impl<'a> PendingRequest<'a> {
    pub(super) fn register(
        pending: &'a StdMutex<Pending>,
        id: u64,
        sender: oneshot::Sender<Result<Value, AiError>>,
    ) -> Result<Self, AiError> {
        let mut entries = pending.lock().map_err(|_| AiError::TemporaryUnavailable)?;
        // Do not replace a live owner even if the request counter ever wraps.
        if entries.contains_key(&id) {
            return Err(AiError::TemporaryUnavailable);
        }
        entries.insert(id, sender);
        Ok(Self { pending, id })
    }
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.pending.lock() {
            entries.remove(&self.id);
        }
    }
}
