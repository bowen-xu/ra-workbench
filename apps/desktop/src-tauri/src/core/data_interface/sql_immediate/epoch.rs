//! The retained Database instance pins one immutable SQL/log owner across a mutation.
use super::*;
impl Database {
    pub(crate) fn check_catalog_epoch(
        &self,
        operation: Uuid,
        expected: Uuid,
        phase: &'static str,
    ) -> Result<(), SqlImmediateFailure> {
        if operation.is_nil() || expected.is_nil() {
            return Err(SqlImmediateFailure::Domain(
                research_agent_core::conversation::CatalogError::InvalidInput,
            ));
        }
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| super::unmanaged(operation))?;
        let state = owner
            .inspection_guard()
            .map_err(|error| super::admission_error(operation, error, None))?
            .snapshot();
        if state.epoch != expected {
            return Err(SqlImmediateFailure::Conflict(Box::new(super::diagnostic(
                operation,
                ReadFence {
                    epoch: state.epoch,
                    position: state.materialized_position,
                },
                phase,
                "data_epoch_changed",
            ))));
        }
        Ok(())
    }
}
