//! Application-owned discard supervision; frontend cancellation is not ownership.
use super::*;
use crate::agent::writing_submission::WritingSubmissionService;
use crate::agent::{
    repository::execution_workspaces::discard::Request,
    tasks::TaskRegistryError,
    workspace::LeaseError,
    writing_submission::workspace::{Error, discard},
};

impl AgentSessionService {
    /// Native confirmation must supply the exact reviewed identity and inventory.
    /// Does not stop an active turn, implicitly accept edits, or restart a model.
    pub async fn discard_workspace(
        self: &Arc<Self>,
        writing: WritingSubmissionService,
        request: Request,
    ) -> Result<(), Error> {
        let operation = request.clone();
        self.discard_workspace_with(request, move || discard::execute(writing, operation))
            .await
    }

    async fn discard_workspace_with<F, Fut>(
        self: &Arc<Self>,
        request: Request,
        operation: F,
    ) -> Result<(), Error>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), Error>> + Send,
    {
        request.validate()?;
        let service = self.clone();
        tokio::spawn(async move {
            let id = ConversationId::from_uuid(request.binding.conversation_id);
            let _admission = service.tasks.reserve_idle(id).map_err(|e| match e {
                TaskRegistryError::Active | TaskRegistryError::Capacity => {
                    Error::Lease(LeaseError::Busy)
                }
                _ => Error::Storage,
            })?;
            let slot = service.slot(id, true).map_err(|_| Error::Storage)?;
            let mut owner = slot.lock().await;
            if owner.is_none() {
                // Inspect persisted history without automatically interrupting it.
                // A missing conversation is allowed only for a completed receipt;
                // the storage operation rejects all other missing identities.
                match DurableConversation::resume(service.store.clone(), fresh_lease(id)).await {
                    Ok(session) => *owner = Some(session),
                    Err(SessionError::Storage(StoreError::NotFound { .. })) => {}
                    Err(_) => return Err(Error::RecoveryRequired),
                }
            }
            if owner
                .as_ref()
                .is_some_and(|s| s.needs_recovery() || s.projection().active_turn().is_some())
            {
                return Err(LeaseError::Busy.into());
            }
            // Keep session, task admission and the application-owned worker alive
            // until both files and database have settled, including uncertain errors.
            operation().await
        })
        .await
        .map_err(|_| Error::RecoveryRequired)?
    }
}

#[cfg(test)]
#[path = "workspace_discard/tests.rs"]
mod tests;
