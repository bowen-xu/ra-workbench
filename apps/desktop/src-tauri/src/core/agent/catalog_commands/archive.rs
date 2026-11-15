//! Proof-only archive and restore; current metadata is an independent read.
use super::SqliteConversationCatalog;
pub use super::mutation::{
    CatalogMutationCommandError as ArchiveCommandError, CatalogMutationProofDto as ArchiveProofDto,
    CatalogMutationStatusResponse as ArchiveStatusResponse,
};
use research_agent_core::conversation::{ArchiveConversation, ConversationCatalog, ConversationId};
use serde::Deserialize;
use uuid::Uuid;

const FAMILY: &str = "agent.conversation.archive";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveRequest {
    pub operation_id: Uuid,
    pub id: Uuid,
    pub archived: bool,
    #[serde(deserialize_with = "required_watermark")]
    pub expected_watermark: Option<u64>,
}
fn required_watermark<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(decoder)
}
impl ArchiveRequest {
    fn decode(self) -> Result<ArchiveConversation, ArchiveCommandError> {
        let command = ArchiveConversation {
            operation_id: self.operation_id,
            id: ConversationId::from_uuid(self.id),
            archived: self.archived,
            expected_watermark: self.expected_watermark,
        };
        command
            .validate()
            .map_err(|error| ArchiveCommandError::Domain(error.into()))?;
        Ok(command)
    }
}

pub(crate) async fn execute_archive(
    catalog: &SqliteConversationCatalog,
    request: ArchiveRequest,
) -> Result<ArchiveProofDto, ArchiveCommandError> {
    Ok(ArchiveProofDto::new(
        catalog.set_archived(request.decode()?).await?,
        FAMILY,
    ))
}

pub(crate) async fn execute_archive_status(
    catalog: &SqliteConversationCatalog,
    request: ArchiveRequest,
) -> Result<ArchiveStatusResponse, ArchiveCommandError> {
    let command = request.decode()?;
    let operation = command.operation_id;
    let id = *command.id.as_uuid();
    let status = catalog.archive_status(command).await?;
    Ok(ArchiveStatusResponse::new(operation, id, status, FAMILY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn archive_wire_requires_original_identity_and_exact_watermark_shape() {
        let request = json!({"operationId":Uuid::new_v4(),"id":Uuid::new_v4(),"archived":true,"expectedWatermark":0});
        assert!(
            serde_json::from_value::<ArchiveRequest>(request.clone())
                .unwrap()
                .decode()
                .is_ok()
        );
        for (key, value) in [
            ("operationId", json!(Uuid::nil())),
            ("expectedWatermark", json!(null)),
            ("expectedWatermark", json!(9_007_199_254_740_992u64)),
        ] {
            let mut bad = request.clone();
            bad[key] = value;
            assert!(
                serde_json::from_value::<ArchiveRequest>(bad)
                    .unwrap()
                    .decode()
                    .is_err()
            );
        }
        let mut restore = request;
        restore["archived"] = json!(false);
        assert!(
            serde_json::from_value::<ArchiveRequest>(restore.clone())
                .unwrap()
                .decode()
                .is_err()
        );
        restore["expectedWatermark"] = json!(null);
        assert!(
            serde_json::from_value::<ArchiveRequest>(restore.clone())
                .unwrap()
                .decode()
                .is_ok()
        );
        for archived in [true, false] {
            let mut missing = restore.clone();
            missing["archived"] = json!(archived);
            missing.as_object_mut().unwrap().remove("expectedWatermark");
            assert!(serde_json::from_value::<ArchiveRequest>(missing).is_err());
        }
        assert!(
            serde_json::from_value::<ArchiveRequest>(json!({"id":Uuid::new_v4(),"archived":true}))
                .is_err()
        );
    }
}
