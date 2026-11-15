//! Catalog rename is a typed SQL transaction, not a journal event or view read.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, SqlImmediateStatus, SqlMutationError, Transaction, receipts,
};
use research_agent_core::conversation::RenameConversation;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.conversation.rename";
const VERSION: u32 = 1;

fn identity(command: &RenameConversation) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    if command.operation_id.is_nil() || command.id.as_uuid().is_nil() {
        return Err(SqlImmediateFailure::Invalid(Box::new(
            SqlImmediateDiagnostic::for_operation(
                command.operation_id,
                None,
                "validate",
                "invalid_rename_identity",
            ),
        )));
    }
    command.validate().map_err(SqlImmediateFailure::Domain)?;
    // Freeze all four intent fields. Trimming applies to the business value,
    // never to operation identity; even whitespace changes need a new ID.
    #[derive(Serialize)]
    struct Frozen<'a> {
        kind: &'static str,
        version: u32,
        operation_id: Uuid,
        conversation_id: Uuid,
        expected_title: &'a str,
        title: &'a str,
    }
    let bytes = serde_json::to_vec(&Frozen {
        kind: KIND,
        version: VERSION,
        operation_id: command.operation_id,
        conversation_id: *command.id.as_uuid(),
        expected_title: &command.expected_title,
        title: &command.title,
    })
    .map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            command.operation_id,
            None,
            "validate",
            "rename_encoding_failed",
        )))
    })?;
    Ok(ReceiptIdentity {
        operation_id: command.operation_id,
        kind: KIND,
        version: VERSION,
        fingerprint: Sha256::digest(bytes).into(),
    })
}

impl Database {
    pub(crate) fn rename_conversation(
        &self,
        command: &RenameConversation,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.rename_conversation_owned(
            command,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn rename_conversation_owned(
        &self,
        command: &RenameConversation,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = identity(command)?;
        self.execute_sql(
            command.operation_id,
            |tx, fence| register(tx, command, &identity, fence),
            #[cfg(test)]
            fault,
        )
    }

    /// Observational even if blocked or closed; never reads the current title,
    /// drains pending records, repairs the owner or repeats a mutation.
    pub(crate) fn rename_status(
        &self,
        command: &RenameConversation,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&identity(command)?)
    }
}

fn register(
    tx: &Transaction<'_>,
    command: &RenameConversation,
    identity: &ReceiptIdentity,
    fence: ReadFence,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation_id = identity.operation_id;
    receipts::check_state(tx, operation_id, fence, "prepare")?;
    receipts::check_cross_kind(tx, operation_id, fence, "prepare")?;
    // The receipt proves this intent, independent of the current title or even
    // later deletion. Replay must not perform fresh CAS, clock reads or writes.
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "prepare")? {
        return Ok(receipt);
    }
    crate::agent::repository::check_rename_in(tx, command)
        .map_err(|error| mutation_error(operation_id, fence, "validate", error))?;
    let updated_at: String = tx
        .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|error| receipts::storage(operation_id, fence, "prepare", &error))?;
    crate::agent::repository::rename_in(tx, command, &updated_at)
        .map_err(|error| mutation_error(operation_id, fence, "mutate", error))?;
    receipts::insert(tx, identity, fence)
}

fn mutation_error(
    operation_id: Uuid,
    fence: ReadFence,
    phase: &'static str,
    error: SqlMutationError,
) -> SqlImmediateFailure {
    match error {
        SqlMutationError::Domain(error) => SqlImmediateFailure::Domain(error),
        SqlMutationError::Sql(error) => receipts::storage(operation_id, fence, phase, &error),
    }
}

#[cfg(test)]
#[path = "rename/tests.rs"]
mod tests;
