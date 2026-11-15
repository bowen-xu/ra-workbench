//! One owner performs capacity checks and proof-gated maintenance. No SQL,
//! timers, directory polling or external side effects live in this policy.
use super::*;

impl<P: Projector> State<P> {
    pub(super) fn maybe_reclaim(&mut self, force: bool) -> CoordinatorResult<bool> {
        let (due, extent) = self.log.maintenance_hint()?;
        if !force
            && (!due
                || self.last_reclaim_extent.is_some_and(|last| {
                    extent < last.saturating_add(self.log.maintenance_step_bytes())
                        && self.materialized.saturating_sub(self.last_reclaim_position)
                            < self.config.reclaim_after_records as u64
                }))
        {
            return Ok(false);
        }
        self.materialize_through(self.accepted)?;
        self.last_reclaim_extent = Some(extent);
        self.last_reclaim_position = self.materialized;
        let fence = self.accepted_fence();
        let proof = match self.projector.reclaim_fence(fence) {
            Ok(proof) => proof,
            Err(error) => {
                self.blocked = true;
                return Err(projector_error("reclaim_fence", fence.position, error));
            }
        };
        let Some(proof) = proof else {
            return Ok(false);
        };
        if let Err(error) = self.validate_reclaim(proof) {
            self.blocked = true;
            return Err(error);
        }
        let before = self.log.head()?.compacted_position;
        if proof.through_position <= before {
            return Ok(false);
        }
        if let Err(error) = self.log.compact_prefix(proof.through_position, true) {
            self.blocked = true;
            return Err(error.into());
        }
        let usage = match self.log.reclaim_files() {
            Ok(usage) => usage,
            Err(error) => {
                self.blocked = true;
                return Err(error.into());
            }
        };
        self.last_reclaim_extent = Some(usage.managed_file_extent_bytes);
        Ok(true)
    }
    pub(super) fn validate_reclaim(&self, proof: ReclaimFence) -> CoordinatorResult<()> {
        if proof.epoch != self.epoch {
            return Err(CoordinatorError::EpochMismatch);
        }
        if proof.through_position > self.materialized
            || proof.through_position > proof.sql_durable_position
            || proof.through_position > proof.retention_safe_position
        {
            return Err(CoordinatorError::InvalidFence);
        }
        Ok(())
    }
    pub(super) fn append_prepared(
        &mut self,
        operation_id: Uuid,
        position: u64,
        payload: &[u8],
        durability: Durability,
    ) -> CoordinatorResult<()> {
        let sync = durability == Durability::Device;
        let first = self.log.append(position, payload, sync);
        match first {
            Ok(()) => return Ok(()),
            Err(error @ LogError::CapacityNotAccepted { .. }) => {
                // No Engine::write was entered. Drain only older accepted data,
                // obtain the projector's proof, then retry the same frozen bytes
                // once. Unknown outcomes never enter this retry branch.
                match self.maybe_reclaim(true) {
                    Ok(true) => {}
                    Ok(false) => return Err(not_accepted(operation_id, position, error.into())),
                    Err(cause) => return Err(not_accepted(operation_id, position, cause)),
                }
            }
            Err(error) => return Err(self.append_error(operation_id, position, error)),
        }
        self.log
            .append(position, payload, sync)
            .map_err(|error| self.append_error(operation_id, position, error))
    }
    fn append_error(
        &mut self,
        operation_id: Uuid,
        position: u64,
        error: LogError,
    ) -> CoordinatorError {
        if matches!(
            error,
            LogError::Engine { .. } | LogError::EnginePanic { .. }
        ) {
            self.blocked = true;
            CoordinatorError::CommitUnknown {
                operation_id,
                position,
                source: error,
            }
        } else {
            if matches!(error, LogError::Poisoned | LogError::Corrupt(_)) {
                self.blocked = true;
            }
            not_accepted(operation_id, position, error.into())
        }
    }
}
fn not_accepted(operation_id: Uuid, position: u64, cause: CoordinatorError) -> CoordinatorError {
    CoordinatorError::NotAccepted {
        operation_id,
        position,
        cause: Box::new(cause),
    }
}
