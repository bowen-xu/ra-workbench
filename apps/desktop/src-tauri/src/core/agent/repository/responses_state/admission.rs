//! Check the durable model intent in the same transaction as the checkpoint.
#[cfg(test)]
use super::ResponsesReceipt;
use super::ResponsesStateScope;
use research_agent_core::{
    model::ModelError,
    protocol::{AgentEvent, EventPayload},
};
use rusqlite::{Connection, OptionalExtension, params};

#[cfg(test)]
pub(super) fn require_running_attempt(
    connection: &Connection,
    scope: &ResponsesStateScope,
    receipt: &ResponsesReceipt,
) -> Result<(), ModelError> {
    require_running_request(
        connection,
        scope,
        receipt.request_id,
        &receipt.requested_model,
    )
}

pub(super) fn require_running_request(
    connection: &Connection,
    scope: &ResponsesStateScope,
    request_id: uuid::Uuid,
    requested_model: &research_agent_core::protocol::ModelIdentity,
) -> Result<(), ModelError> {
    // Phase/text events do not advance model admission. The last control event
    // must be this exact request's start, not another attempt or a terminal.
    let encoded: Option<String> = connection
        .query_row(
            "select payload_json from agent_items
             where conversation_id=?1 and schema_version=1
               and json_extract(payload_json,'$.payload.turnId')=?2
               and json_extract(payload_json,'$.payload.type') in
                 ('turnStarted','turnFinished','modelRequestStarted','modelRequestFinished',
                  'modelRequestFailed','modelCompactionFinished')
             order by ordinal desc limit 1",
            params![scope.conversation_id.to_string(), scope.turn_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| ModelError::Unavailable)?;
    let event: AgentEvent = serde_json::from_str(&encoded.ok_or(ModelError::InvalidRequest)?)
        .map_err(|_| ModelError::InvalidStream)?;
    if *event.conversation_id().as_uuid() != scope.conversation_id {
        return Err(ModelError::InvalidStream);
    }
    // A receipt from covered history cannot repopulate a replacement window,
    // even when its request remains the last model intent in the journal.
    if super::super::context_windows::read_active(connection, scope.conversation_id)
        .map_err(|_| ModelError::InvalidStream)?
        .is_some_and(|checkpoint| event.seq() <= checkpoint.source_watermark)
    {
        return Err(ModelError::InvalidRequest);
    }
    match event.payload() {
        EventPayload::ModelRequestStarted {
            turn_id,
            attempt_id,
            model,
        } if *turn_id == scope.turn_id
            && *attempt_id == request_id
            && *model == scope.model
            && model == requested_model =>
        {
            Ok(())
        }
        _ => Err(ModelError::InvalidRequest),
    }
}

pub(super) fn validate_scope(
    db: &Connection,
    scope: &ResponsesStateScope,
) -> Result<(), ModelError> {
    let input:Option<(String,String)>=db.query_row("select s.request_id,s.input_json from agent_submissions s join agent_conversations c on c.id=s.conversation_id where s.conversation_id=?1 and s.turn_id=?2 and s.context_snapshot_id=?3 and s.schema_version=1 and c.archived=0",
        params![scope.conversation_id.to_string(),scope.turn_id.to_string(),scope.context_snapshot_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|ModelError::Unavailable)?;
    let (request, encoded) = input.ok_or(ModelError::InvalidRequest)?;
    let input: research_agent_core::submission::SubmissionInput =
        serde_json::from_str(&encoded).map_err(|_| ModelError::InvalidStream)?;
    let images = if input.text.trim().is_empty() {
        let request = uuid::Uuid::parse_str(&request).map_err(|_| ModelError::InvalidStream)?;
        super::super::submissions::read_image_manifest(db, scope.conversation_id, request)
            .map_err(|_| ModelError::InvalidStream)?
    } else {
        vec![]
    };
    input
        .validate_with_image_references(&images)
        .map_err(|_| ModelError::InvalidStream)?;
    if input.model != scope.model {
        return Err(ModelError::InvalidRequest);
    }
    Ok(())
}
