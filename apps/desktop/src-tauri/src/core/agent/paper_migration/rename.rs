//! One stable rename phase per durable migration mapping, distinct from its ID.
use super::{Error, SqliteConversationCatalog};
use research_agent_core::conversation::{
    CatalogError, CatalogMutationError, ConversationCatalog, ConversationId, RenameConversation,
};
use uuid::Uuid;

fn intent(conversation: Uuid, title: &str) -> RenameConversation {
    RenameConversation {
        operation_id: Uuid::new_v5(&conversation, b"agent.paper_migration.rename:v1"),
        id: ConversationId::from_uuid(conversation),
        expected_title: "新对话".into(),
        title: if title.trim().is_empty() {
            "文献阅读"
        } else {
            title.trim()
        }
        .to_owned(),
    }
}
pub(super) async fn apply(
    catalog: &SqliteConversationCatalog,
    conversation: Uuid,
    title: &str,
) -> Result<(), Error> {
    let command = intent(conversation, title);
    // Naming is auxiliary to importing old history. Keep an invalid old title
    // verbatim in metadata and retain the current name, without a fake proof.
    // Invalid identities and every storage failure still stop the operation.
    if !command.id.as_uuid().is_nil()
        && !command.operation_id.is_nil()
        && matches!(command.validate(), Err(CatalogError::InvalidInput))
    {
        return Ok(());
    }
    match catalog.rename(command).await {
        Ok(_) => Ok(()),
        // Migration supplies a default title; a user's newer metadata wins.
        Err(CatalogMutationError::Domain(CatalogError::Conflict)) => Ok(()),
        Err(error) => Err(Error::Mutation(error.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_rename_phase_is_stable_and_does_not_reuse_the_mapping_identity() {
        let conversation = Uuid::from_u128(11);
        let first = intent(conversation, " Legacy title ");
        assert_eq!(first, intent(conversation, " Legacy title "));
        assert_eq!(first.title, "Legacy title");
        assert_eq!(first.expected_title, "新对话");
        assert_ne!(first.operation_id, conversation);
        assert_ne!(
            first.operation_id,
            intent(Uuid::from_u128(12), "Legacy title").operation_id
        );
        assert_ne!(
            first.operation_id,
            Uuid::new_v5(&conversation, b"agent.paper_migration.archive:v1")
        );
        assert_eq!(intent(conversation, " ").title, "文献阅读");
    }
}
