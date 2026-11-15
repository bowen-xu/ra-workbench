use super::super::context_windows::initial_metadata::{self, Snapshot};
use super::*;
impl SqliteContextCheckpointStore {
    pub(crate) async fn initial_metadata(
        &self,
        conversation: Uuid,
        window: Uuid,
        through: u64,
    ) -> Result<Option<String>, CheckpointStoreError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let db = connection
                .lock()
                .map_err(|_| CheckpointStoreError::Unavailable)?;
            initial_metadata::read(&db, conversation, window, through)
                .map(|snapshot| snapshot.map(|snapshot| snapshot.body))
                .map_err(map_error)
        })
        .await
        .map_err(|_| CheckpointStoreError::Unavailable)?
    }
    pub(crate) async fn record_initial_metadata(
        &self,
        snapshot: Snapshot,
        signal: crate::agent::cancellation::AgentCancellation,
    ) -> Result<String, CheckpointStoreError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = connection
                .lock()
                .map_err(|_| CheckpointStoreError::Unavailable)?;
            initial_metadata::record(&mut db, &snapshot, &signal).map_err(map_error)
        })
        .await
        .map_err(|_| CheckpointStoreError::Unavailable)?
    }
}
