//! Read-only recovery candidates. Startup still uses the confirmed queue command.
use super::{
    writing_commands::SubmissionPreview, writing_queue::CommandError,
    writing_submission::WritingSubmissionService,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub preview: SubmissionPreview,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub items: Vec<Item>,
    pub next_cursor: Option<Uuid>,
}
#[tauri::command]
pub async fn agent_list_unstarted_writing_queue_admissions(
    service: State<'_, WritingSubmissionService>,
    request: Request,
) -> Result<Page, CommandError> {
    list(&service, request).await
}
pub(crate) async fn list(
    service: &WritingSubmissionService,
    request: Request,
) -> Result<Page, CommandError> {
    if request.conversation_id.is_nil()
        || request.manuscript_id.is_nil()
        || request.after.is_some_and(|id| id.is_nil())
        || request.limit == 0
        || request.limit > 64
    {
        return Err(CommandError::Invalid);
    }
    let records = service
        .list_unstarted_queue_admissions(
            request.manuscript_id,
            request.conversation_id,
            request.after,
            request.limit,
        )
        .await?;
    let mut items = Vec::with_capacity(records.len());
    for record in records {
        if record.expected_revision > 9_007_199_254_740_991 {
            return Err(CommandError::Corrupt);
        }
        items.push(Item {
            input_id: record.input_id,
            expected_revision: record.expected_revision,
            preview: SubmissionPreview::from_record(record.submission, request.manuscript_id),
        });
    }
    let next_cursor = if items.len() == usize::from(request.limit) {
        items.last().map(|item| item.input_id)
    } else {
        None
    };
    Ok(Page {
        schema_version: 1,
        conversation_id: request.conversation_id,
        manuscript_id: request.manuscript_id,
        items,
        next_cursor,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_accepts_only_scoped_pagination() {
        let value = serde_json::json!({"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"after":null,"limit":20});
        assert!(serde_json::from_value::<Request>(value.clone()).is_ok());
        let mut bad = value;
        bad["start"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Request>(bad).is_err());
    }
}
