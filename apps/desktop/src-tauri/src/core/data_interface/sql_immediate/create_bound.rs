//! The empty conversation, initial host association and proof commit together.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, SqlImmediateStatus, SqlMutationError, Transaction, receipts,
};
use research_agent_core::conversation::CreateBoundConversation;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.conversation.create_bound";
const VERSION: u32 = 1;

fn identity(command: &CreateBoundConversation) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    if command.operation_id.is_nil() || command.id.as_uuid().is_nil() {
        return Err(SqlImmediateFailure::Invalid(Box::new(
            SqlImmediateDiagnostic::for_operation(
                command.operation_id,
                None,
                "validate",
                "invalid_create_bound_identity",
            ),
        )));
    }
    command.validate().map_err(SqlImmediateFailure::Domain)?;
    #[derive(Serialize)]
    struct Frozen<'a> {
        kind: &'static str,
        version: u32,
        operation_id: Uuid,
        conversation_id: Uuid,
        binding: &'a research_agent_core::conversation::ConversationBinding,
    }
    let bytes = serde_json::to_vec(&Frozen {
        kind: KIND,
        version: VERSION,
        operation_id: command.operation_id,
        conversation_id: *command.id.as_uuid(),
        binding: &command.binding,
    })
    .map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            command.operation_id,
            None,
            "validate",
            "create_bound_encoding_failed",
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
    pub(crate) fn create_bound_conversation(
        &self,
        command: &CreateBoundConversation,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.create_bound_conversation_owned(
            command,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn create_bound_conversation_owned(
        &self,
        command: &CreateBoundConversation,
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

    /// An original proof does not establish that its target is currently live.
    /// Inspection never repeats creation, drains pending data or reads metadata.
    pub(crate) fn bound_conversation_creation_status(
        &self,
        command: &CreateBoundConversation,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&identity(command)?)
    }
}

fn register(
    tx: &Transaction<'_>,
    command: &CreateBoundConversation,
    identity: &ReceiptIdentity,
    fence: ReadFence,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation_id = identity.operation_id;
    receipts::check_state(tx, operation_id, fence, "prepare")?;
    receipts::check_cross_kind(tx, operation_id, fence, "prepare")?;
    // Retain the existing create endpoint's live target check on retries.
    // Status remains historical; neither it nor a proof authorizes model work.
    crate::agent::repository::check_create_bound_target_in(tx, command)
        .map_err(|error| mutation_error(operation_id, fence, "authorize", error))?;
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "prepare")? {
        return Ok(receipt);
    }
    crate::agent::repository::check_create_bound_in(tx, command)
        .map_err(|error| mutation_error(operation_id, fence, "validate", error))?;
    let updated_at: String = tx
        .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|error| receipts::storage(operation_id, fence, "prepare", &error))?;
    crate::agent::repository::create_bound_in(tx, command, &updated_at)
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
#[path = "create_bound/tests.rs"]
mod tests;
