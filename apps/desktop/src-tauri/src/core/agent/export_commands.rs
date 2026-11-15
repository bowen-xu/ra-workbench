//! Read-only transcript boundary. It cannot resume a turn or write a file.
use super::repository::{SnapshotExportError, SqliteConversationCatalog};
use research_agent_core::conversation::{ConversationExport, ConversationId};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExportCommandError {
    InvalidInput,
    NotFound,
    TooLarge,
    Corrupt,
    Unavailable,
}

impl From<SnapshotExportError> for ExportCommandError {
    fn from(value: SnapshotExportError) -> Self {
        match value {
            SnapshotExportError::NotFound => Self::NotFound,
            SnapshotExportError::TooLarge => Self::TooLarge,
            SnapshotExportError::Corrupt => Self::Corrupt,
            SnapshotExportError::Unavailable => Self::Unavailable,
        }
    }
}

impl ExportRequest {
    pub(super) fn identity(self) -> Result<ConversationId, ExportCommandError> {
        if self.conversation_id.is_nil() {
            return Err(ExportCommandError::InvalidInput);
        }
        Ok(ConversationId::from_uuid(self.conversation_id))
    }
}

#[tauri::command]
pub async fn agent_export_conversation_snapshot(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ExportRequest,
) -> Result<ConversationExport, ExportCommandError> {
    Ok(catalog.export_snapshot(request.identity()?).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_request_accepts_only_an_explicit_non_nil_conversation() {
        let id = Uuid::from_u128(1);
        let valid = serde_json::json!({"conversationId": id});
        assert_eq!(
            serde_json::from_value::<ExportRequest>(valid.clone())
                .unwrap()
                .identity()
                .unwrap()
                .as_uuid(),
            &id
        );
        for invalid in [
            serde_json::json!({}),
            serde_json::json!({"conversationId": "invalid"}),
            serde_json::json!({"conversationId": id, "destinationPath": "/tmp/file.json"}),
            serde_json::json!({"conversationId": id, "resume": true}),
        ] {
            assert!(serde_json::from_value::<ExportRequest>(invalid).is_err());
        }
        assert_eq!(
            ExportRequest {
                conversation_id: Uuid::nil()
            }
            .identity(),
            Err(ExportCommandError::InvalidInput)
        );
    }
}
