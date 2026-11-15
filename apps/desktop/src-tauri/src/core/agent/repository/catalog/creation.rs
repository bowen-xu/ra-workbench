//! Creation and initial host association share one transaction, without a runtime lease.
use super::mutation::{mutation_error, proof, status, worker_failure};
use super::*;
use crate::data_interface::SqlMutationError;
use research_agent_core::conversation::CatalogMutationError;
use rusqlite::Transaction;

impl SqliteConversationCatalog {
    #[cfg(test)]
    pub(crate) async fn create_bound_legacy_for_tests(
        &self,
        id: ConversationId,
        binding: ConversationBinding,
    ) -> Result<ConversationSummary, CatalogError> {
        if self.connection.logged() || id.as_uuid().is_nil() {
            return Err(CatalogError::InvalidInput);
        }
        validate_binding(&binding)?;
        self.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
            let target = binding_target_query(binding.kind);
            if !tx.query_row(target, [&binding.resource_id], |row| row.get::<_, bool>(0)).map_err(db_error)? {
                return Err(CatalogError::NotFound);
            }
            let key = id.as_uuid().to_string();
            let existing = tx.query_row(
                "select id, title, archived, durable_count, created_at, updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations where id = ?1",
                [&key], read_row,
            ).optional().map_err(db_error)?;
            let summary = if let Some(row) = existing {
                let matched: bool = tx.query_row(
                    "select exists(select 1 from agent_conversation_bindings where conversation_id = ?1 and resource_kind = ?2 and resource_id = ?3)",
                    params![key, binding_kind(binding.kind), binding.resource_id], |row| row.get(0),
                ).map_err(db_error)?;
                if !matched { return Err(CatalogError::Conflict); }
                decode(row)?
            } else {
                tx.execute("insert into agent_conversations(id, title) values (?1, '新对话')", [&key]).map_err(db_error)?;
                tx.execute(
                    "insert into agent_conversation_bindings(conversation_id, resource_kind, resource_id) values (?1, ?2, ?3)",
                    params![key, binding_kind(binding.kind), binding.resource_id],
                ).map_err(db_error)?;
                decode(tx.query_row(
                    "select id, title, archived, durable_count, created_at, updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations where id = ?1",
                    [&key], read_row,
                ).map_err(db_error)?)?
            };
            tx.commit().map_err(db_error)?;
            Ok(summary)
        }).await
    }
}

impl SqliteConversationCatalog {
    pub(super) async fn create_owned(
        &self,
        command: CreateBoundConversation,
    ) -> Result<CreateBoundProof, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let receipt = self
            .worker_retention
            .spawn(move || connection.create_bound_conversation(&command))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    true,
                    "create-bound-worker",
                    "create_bound_worker_result_unknown",
                )
            })?
            .map_err(mutation_error)?;
        Ok(proof(id, receipt))
    }
    pub(super) async fn inspect_creation(
        &self,
        command: CreateBoundConversation,
    ) -> Result<CreateBoundStatus, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let value = self
            .worker_retention
            .spawn(move || connection.bound_conversation_creation_status(&command))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    false,
                    "create-bound-status-worker",
                    "create_bound_status_worker_failed",
                )
            })?
            .map_err(mutation_error)?;
        Ok(status(id, value))
    }
}

/// Live resource checks precede receipt replay, even if the original creation committed.
pub(crate) fn check_create_bound_target_in(
    tx: &Transaction<'_>,
    command: &CreateBoundConversation,
) -> Result<(), SqlMutationError> {
    command.validate()?;
    if !tx.query_row(
        binding_target_query(command.binding.kind),
        [&command.binding.resource_id],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(CatalogError::NotFound.into());
    }
    Ok(())
}
/// A fresh operation may reuse only an already matching association; an empty
/// unrelated conversation is never adopted. Existing metadata is not rewritten.
pub(crate) fn check_create_bound_in(
    tx: &Transaction<'_>,
    command: &CreateBoundConversation,
) -> Result<(), SqlMutationError> {
    check_create_bound_target_in(tx, command)?;
    let key = command.id.as_uuid().to_string();
    let exists: bool = tx.query_row(
        "select exists(select 1 from agent_conversations where id=?1)",
        [&key],
        |row| row.get(0),
    )?;
    if exists {
        let matched:bool=tx.query_row("select exists(select 1 from agent_conversation_bindings where conversation_id=?1 and resource_kind=?2 and resource_id=?3)",params![key,binding_kind(command.binding.kind),command.binding.resource_id],|row|row.get(0))?;
        if !matched {
            return Err(CatalogError::Conflict.into());
        }
    }
    Ok(())
}
/// Only the facade supplies this transaction and trusted timestamp. A matching
/// existing conversation is a no-op; tombstone triggers still reject resurrection.
pub(crate) fn create_bound_in(
    tx: &Transaction<'_>,
    command: &CreateBoundConversation,
    updated_at: &str,
) -> Result<(), SqlMutationError> {
    check_create_bound_in(tx, command)?;
    let key = command.id.as_uuid().to_string();
    let exists: bool = tx.query_row(
        "select exists(select 1 from agent_conversations where id=?1)",
        [&key],
        |row| row.get(0),
    )?;
    if !exists {
        tx.execute("insert into agent_conversations(id,title,created_at,updated_at) values (?1,'新对话',?2,?2)",params![key,updated_at])?;
        tx.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id,created_at) values (?1,?2,?3,?4)",params![key,binding_kind(command.binding.kind),command.binding.resource_id,updated_at])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "creation/managed_tests.rs"]
mod managed_tests;
