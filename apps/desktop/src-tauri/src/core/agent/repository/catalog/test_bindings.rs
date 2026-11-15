//! Old-schema fixtures can seed additional historical host associations.
//! Production associations belong to their creating or moving business owner.
use super::*;

impl SqliteConversationCatalog {
    pub(crate) async fn bind_legacy_for_tests(
        &self,
        id: ConversationId,
        binding: ConversationBinding,
    ) -> Result<(), CatalogError> {
        if self.connection.logged() {
            return Err(CatalogError::InvalidInput);
        }
        validate_binding(&binding)?;
        self.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
            let exists: bool = tx.query_row("select exists(select 1 from agent_conversations where id = ?1)", [id.as_uuid().to_string()], |row| row.get(0)).map_err(db_error)?;
            if !exists { return Err(CatalogError::NotFound); }
            let exists: bool = tx.query_row(binding_target_query(binding.kind), [&binding.resource_id], |row| row.get(0)).map_err(db_error)?;
            if !exists { return Err(CatalogError::NotFound); }
            tx.execute("insert into agent_conversation_bindings(conversation_id, resource_kind, resource_id) values (?1, ?2, ?3) on conflict do nothing",
                params![id.as_uuid().to_string(), binding_kind(binding.kind), binding.resource_id]).map_err(db_error)?;
            tx.commit().map_err(db_error)
        }).await
    }
}
