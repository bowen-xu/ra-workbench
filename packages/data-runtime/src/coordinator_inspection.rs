//! Read cached owner state under the admission lock without doing recovery work.
use super::*;

/// Last-confirmed in-memory heads, not an operation receipt or commit result.
/// When blocked, either durable store may be ahead of these cached positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InspectionSnapshot {
    pub epoch: Uuid,
    pub accepted_position: u64,
    pub materialized_position: u64,
    /// Exact retained serialized log-record bytes, not command or heap size.
    /// A blocked owner retains them even if an unconfirmed SQL commit occurred.
    pub pending_bytes: usize,
    pub blocked: bool,
    pub closed: bool,
}

/// Holds the existing admission mutex. Only cached metadata is exposed; callers
/// cannot access the projector, commands, engine or mutable coordinator state.
pub struct InspectionGuard<'a, P: Projector> {
    state: MutexGuard<'a, State<P>>,
}
impl<P: Projector> InspectionGuard<'_, P> {
    pub fn snapshot(&self) -> InspectionSnapshot {
        InspectionSnapshot {
            epoch: self.state.epoch,
            accepted_position: self.state.accepted,
            materialized_position: self.state.materialized,
            pending_bytes: self.state.pending_bytes,
            blocked: self.state.blocked,
            closed: self.state.closed,
        }
    }
}
impl<P: Projector> Coordinator<P> {
    /// Inspect even a blocked or closed owner without materializing, syncing,
    /// reclaiming, consulting the projector/engine, or clearing its block.
    /// Poison still requires recovery. Heads or receipt absence must not be used
    /// as proof that an uncertain operation failed to commit.
    ///
    /// A status adapter may acquire its raw SQL mutex while this guard lives:
    /// lock order is admission inspection, then SQL. Never acquire an outer
    /// maintenance lock while holding this guard.
    pub fn inspection_guard(&self) -> CoordinatorResult<InspectionGuard<'_, P>> {
        Ok(InspectionGuard {
            state: self.lock()?,
        })
    }
}

#[cfg(test)]
#[path = "coordinator_inspection/tests.rs"]
mod tests;
