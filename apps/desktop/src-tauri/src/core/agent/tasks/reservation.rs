//! Exclusive non-running lifecycle operations share task admission, including cleanup.
use super::*;

pub(crate) struct IdleReservation<'a> {
    registry: &'a AgentTaskRegistry,
    id: ConversationId,
}
impl AgentTaskRegistry {
    pub(crate) fn is_closing(&self) -> Result<bool, TaskRegistryError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?
            .closing)
    }
    pub(super) async fn wait_for_reservations(&self) -> Result<(), TaskRegistryError> {
        loop {
            let changed = self.reservation_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .state
                .lock()
                .map_err(|_| TaskRegistryError::Unavailable)?
                .reservations
                .is_empty()
            {
                return Ok(());
            }
            changed.await;
        }
    }
    pub(crate) fn reserve_idle(
        &self,
        id: ConversationId,
    ) -> Result<IdleReservation<'_>, TaskRegistryError> {
        self.reserve_idle_owned(id, false)
    }
    /// A catalog batch does not start tasks; hold all its idle reservations
    /// together even when the frozen batch exceeds the active-task limit.
    pub(crate) fn reserve_idle_for_batch(
        &self,
        id: ConversationId,
    ) -> Result<IdleReservation<'_>, TaskRegistryError> {
        self.reserve_idle_owned(id, true)
    }
    fn reserve_idle_owned(
        &self,
        id: ConversationId,
        batch: bool,
    ) -> Result<IdleReservation<'_>, TaskRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        if state.closing {
            return Err(TaskRegistryError::Closing);
        }
        Self::retain_finished(&mut state);
        if state.tasks.contains_key(&id) || state.reservations.contains(&id) {
            return Err(TaskRegistryError::Active);
        }
        if !batch
            && state.tasks.len()
                + state
                    .reservations
                    .len()
                    .saturating_sub(state.batch_reservations.len())
                >= MAX_TASKS
        {
            return Err(TaskRegistryError::Capacity);
        }
        state.reservations.insert(id);
        if batch {
            state.batch_reservations.insert(id);
        }
        Ok(IdleReservation { registry: self, id })
    }
}
impl Drop for IdleReservation<'_> {
    fn drop(&mut self) {
        // Recover only to release our own reservation; poisoned admission stays closed.
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.reservations.remove(&self.id);
        state.batch_reservations.remove(&self.id);
        self.registry.reservation_changed.notify_waiters();
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn batch_reservations_do_not_consume_unrelated_task_capacity() {
        let registry = AgentTaskRegistry::default();
        let held: Vec<_> = (1..=MAX_TASKS + 10)
            .map(|n| {
                registry
                    .reserve_idle_for_batch(ConversationId::from_uuid(Uuid::from_u128(n as u128)))
                    .unwrap()
            })
            .collect();
        let other = ConversationId::from_uuid(Uuid::from_u128(1000));
        assert!(registry.reserve_idle(other).is_ok());
        assert_eq!(
            registry
                .reserve_idle_for_batch(ConversationId::from_uuid(Uuid::from_u128(1)))
                .err(),
            Some(TaskRegistryError::Active)
        );
        drop(held);
    }
}
