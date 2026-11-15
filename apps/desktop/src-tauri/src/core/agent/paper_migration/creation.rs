//! One creation receipt phase per durable migration mapping. The mapping's
//! compound claim remains atomic; this phase never rebuilds or releases it.
use super::{Error, SqliteConversationCatalog};
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationCatalog, ConversationId, CreateBoundConversation,
};
use uuid::Uuid;

fn intent(id: Uuid, paper_id: &str) -> CreateBoundConversation {
    CreateBoundConversation {
        operation_id: Uuid::new_v5(&id, b"agent.paper_migration.create_bound:v1"),
        id: ConversationId::from_uuid(id),
        binding: ConversationBinding {
            kind: BindingKind::Paper,
            resource_id: paper_id.to_owned(),
        },
    }
}
pub(super) async fn apply(
    catalog: &SqliteConversationCatalog,
    id: Uuid,
    paper_id: &str,
) -> Result<(), Error> {
    catalog
        .create_bound(intent(id, paper_id))
        .await
        .map(|_| ())
        .map_err(|error| Error::Mutation(error.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_phase_is_stable_independent_and_keeps_original_binding() {
        let id = Uuid::from_u128(17);
        let first = intent(id, " paper ");
        assert_eq!(first, intent(id, " paper "));
        assert_ne!(first.operation_id, id);
        assert_ne!(
            first.operation_id,
            Uuid::new_v5(&id, b"agent.paper_migration.rename:v1")
        );
        assert_eq!(first.binding.resource_id, " paper ");
        assert_ne!(first, intent(id, "paper"));
    }
}
