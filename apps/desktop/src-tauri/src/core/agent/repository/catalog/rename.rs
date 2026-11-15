//! Rename is a managed receipt mutation; ordinary metadata reads stay separate.
use super::mutation::{mutation_error, proof, status, worker_failure};
use super::*;
use crate::data_interface::SqlAuthorizationError;
use research_agent_core::conversation::CatalogMutationError;
use rusqlite::Transaction;

impl SqliteConversationCatalog {
    pub(super) async fn rename_owned(
        &self,
        command: RenameConversation,
    ) -> Result<RenameProof, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let receipt = self
            .worker_retention
            .spawn(move || connection.rename_conversation(&command))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    true,
                    "rename-worker",
                    "rename_worker_result_unknown",
                )
            })?
            .map_err(mutation_error)?;
        Ok(proof(id, receipt))
    }
    pub(super) async fn inspect_rename(
        &self,
        command: RenameConversation,
    ) -> Result<RenameStatus, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let value = self
            .worker_retention
            .spawn(move || connection.rename_status(&command))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    false,
                    "rename-status-worker",
                    "rename_status_worker_failed",
                )
            })?
            .map_err(mutation_error)?;
        Ok(status(id, value))
    }

    /// Old schema fixtures explicitly opt out of managed receipts. Production
    /// callers cannot fall back here or fabricate an epoch for a SQL-only owner.
    #[cfg(test)]
    pub(crate) async fn rename_legacy_for_tests(
        &self,
        id: ConversationId,
        expected_title: String,
        title: String,
    ) -> Result<ConversationSummary, CatalogError> {
        if self.connection.logged() {
            return Err(CatalogError::InvalidInput);
        }
        let title = title.trim().to_owned();
        if title.is_empty() || title.chars().count() > 180 || title.contains('\0') {
            return Err(CatalogError::InvalidInput);
        }
        self.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
            let current: String = tx.query_row("select title from agent_conversations where id=?1", [id.as_uuid().to_string()], |row| row.get(0)).optional().map_err(db_error)?.ok_or(CatalogError::NotFound)?;
            if current != title {
                if current != expected_title { return Err(CatalogError::Conflict); }
                tx.execute("update agent_conversations set title=?1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?2", params![title,id.as_uuid().to_string()]).map_err(db_error)?;
            }
            tx.commit().map_err(db_error)
        }).await?;
        self.read(id, None).await
    }
}

/// Fresh writes validate CAS only after the facade has excluded exact receipts.
/// This check runs before the facade reads its trusted clock.
pub(crate) fn check_rename_in(
    tx: &Transaction<'_>,
    command: &RenameConversation,
) -> Result<(), SqlAuthorizationError> {
    command.validate()?;
    let current: String = tx
        .query_row(
            "select title from agent_conversations where id=?1",
            [command.id.as_uuid().to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(CatalogError::NotFound)?;
    if current != command.expected_title {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

/// The facade owns this transaction and prepared time. No clocks, random IDs,
/// runtime leases, model calls or external effects run inside the mutation.
pub(crate) fn rename_in(
    tx: &Transaction<'_>,
    command: &RenameConversation,
    updated_at: &str,
) -> Result<(), SqlAuthorizationError> {
    check_rename_in(tx, command)?;
    let title = command.title.trim();
    if title == command.expected_title {
        return Ok(());
    }
    let changed = tx.execute(
        "update agent_conversations set title=?1,updated_at=?2 where id=?3 and title=?4",
        params![
            title,
            updated_at,
            command.id.as_uuid().to_string(),
            command.expected_title
        ],
    )?;
    if changed != 1 {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "rename/tests.rs"]
mod tests;
