use super::{AppStore, StorageError};

impl AppStore {
    /// Keep resources reachable from this process's editor/undo history alive.
    /// Call under content_read, before returning bytes or a document snapshot;
    /// GC tests membership under content_write. No persisted reference changes.
    pub(crate) fn retain_content_for_session(&self, hash: &str) -> Result<(), StorageError> {
        self.content_session_retained
            .lock()
            .map_err(|_| StorageError::DatabaseBusy)?
            .insert(hash.to_owned());
        Ok(())
    }

    pub(crate) fn is_content_session_retained(&self, hash: &str) -> Result<bool, StorageError> {
        Ok(self
            .content_session_retained
            .lock()
            .map_err(|_| StorageError::DatabaseBusy)?
            .contains(hash))
    }
}
