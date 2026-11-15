//! Re-editing never accepts caller-supplied provenance or reuses a durable input ID.
use super::*;

pub(super) fn restore_origin_sql(
    db: &Connection,
    conversation: Uuid,
    edit: &mut QueueEdit,
) -> Result<(), super::QueueSqlError> {
    let QueueEdit::Restore {
        input,
        source_id,
        paused,
        ..
    } = edit
    else {
        return Ok(());
    };
    let removed: bool = db.query_row(
        "select exists(select 1 from agent_input_queue_receipts where conversation_id=?1 and json_extract(command_json,'$[2][0]')='remove' and json_extract(command_json,'$[2][1]')=?2)",
        params![conversation.to_string(), source_id.to_string()], |row| row.get(0),
    )?;
    if !removed {
        return Err(QueueSqlError::Domain(Error::Queue(QueueError::Conflict)));
    }
    let source: Option<String> = db.query_row(
        "select result_json from agent_input_queue_receipts where conversation_id=?1 and exists(select 1 from json_each(result_json,'$.inputs') where json_extract(value,'$[0]')=?2) order by json_extract(result_json,'$.revision') desc limit 1",
        params![conversation.to_string(), source_id.to_string()], |row| row.get(0),
    ).optional()?;
    let queue = Snapshot::decode(
        &source.ok_or(QueueSqlError::Domain(Error::Queue(QueueError::Conflict)))?,
        conversation,
    )
    .map_err(QueueSqlError::Domain)?;
    let original = queue
        .inputs()
        .iter()
        .find(|item| item.id == *source_id)
        .ok_or(QueueSqlError::Domain(Error::Queue(QueueError::Conflict)))?;
    input.origin.clone_from(&original.origin);
    *paused = queue.input_paused(*source_id);
    Ok(())
}
