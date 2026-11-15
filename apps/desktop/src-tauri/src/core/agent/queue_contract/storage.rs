//! Preserve queue business refusals and the full safe typed-storage identity.
use super::CommandError;
use crate::data_interface::{
    QueueEditFailure, QueueEditIntent, QueueEditScope, SqlImmediateFailure,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStorageError {
    conversation_id: Uuid,
    request_id: Uuid,
    operation_id: Uuid,
    scope: QueueEditScope,
    reason: Box<SqlImmediateFailure>,
}
pub(crate) fn failure(intent: &QueueEditIntent, cause: QueueEditFailure) -> CommandError {
    match cause {
        QueueEditFailure::Domain(cause) => cause.into(),
        QueueEditFailure::Storage(cause) => CommandError::Storage(QueueStorageError {
            conversation_id: intent.conversation_id,
            request_id: intent.request_id,
            operation_id: intent.request_id,
            scope: intent.scope.clone(),
            reason: Box::new(cause),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::input_queue::{QueueEdit, QueueError};
    #[test]
    fn storage_wire_keeps_full_identity_and_original_codes_without_payload() {
        let intent = QueueEditIntent {
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            expected_revision: 0,
            expected_epoch: None,
            scope: QueueEditScope::Context,
            edit: QueueEdit::Pause,
        };
        for (cause, code) in [
            (QueueError::Invalid, "invalid"),
            (QueueError::Conflict, "conflict"),
            (QueueError::Busy, "busy"),
            (QueueError::Paused, "paused"),
            (QueueError::Capacity, "capacity"),
        ] {
            assert_eq!(
                serde_json::to_value(failure(&intent, QueueEditFailure::Domain(cause.into())))
                    .unwrap(),
                serde_json::json!({"code":code})
            );
        }
        use crate::data_interface::IntoDatabase;
        let db = std::sync::Arc::new(std::sync::Mutex::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ))
        .into_database();
        let error = db.edit_input_queue(&intent).unwrap_err();
        let value = serde_json::to_value(failure(&intent, error)).unwrap();
        assert_eq!(value["code"], "QUEUE_STORAGE_FAILURE");
        assert_eq!(value["conversationId"], intent.conversation_id.to_string());
        assert_eq!(value["requestId"], intent.request_id.to_string());
        assert_eq!(value["operationId"], intent.request_id.to_string());
        assert_eq!(value["scope"], serde_json::json!({"kind":"context"}));
        assert_eq!(value["reason"]["code"], "DATA_SQL_UNMANAGED");
        assert_eq!(
            value["reason"]["reason"]["safeCause"],
            "managed_owner_required"
        );
        assert_eq!(value.as_object().unwrap().len(), 6);
    }
}
