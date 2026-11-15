//! Consume a quiescent owner without releasing the engine's directory lock.
use super::*;

impl<P: Projector> Coordinator<P> {
    /// Stop admission, drain and sync through the existing shutdown boundary,
    /// then transfer the same locked engine and projector. This is intended for
    /// an unpublished startup owner before a schema migration, not live clients.
    /// Failure returns no usable owner; its accepted suffix remains recoverable.
    pub fn shutdown_into_parts(self) -> CoordinatorResult<(RaftLog, P)> {
        self.shutdown()?;
        let state = self
            .state
            .into_inner()
            .map_err(|_| CoordinatorError::RecoveryRequired)?;
        Ok((state.log, state.projector))
    }
}
