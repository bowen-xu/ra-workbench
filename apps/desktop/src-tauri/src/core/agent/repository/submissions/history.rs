use super::*;

impl SqliteSubmissionStore {
    pub(super) async fn historical_input(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<HistoricalTurnInput, SubmissionError> {
        if conversation.is_nil() || turn.is_nil() {
            return Err(SubmissionError::InvalidInput);
        }
        self.run(move |db| {
            let tx = db.transaction().map_err(db_error)?;
            let record = read_historical_record(&tx, conversation, turn)?;
            let result = HistoricalTurnInput::from_record(conversation, record);
            tx.commit().map_err(db_error)?;
            Ok(result)
        })
        .await
    }
}
pub(crate) fn read_historical_record(
    db: &Connection,
    conversation: Uuid,
    turn: Uuid,
) -> Result<SubmissionRecord, SubmissionError> {
    if conversation.is_nil() || turn.is_nil() {
        return Err(SubmissionError::InvalidInput);
    }
    if let Some(record) = read_turn(db, conversation, turn)? {
        return Ok(record);
    }
    let source: Option<(String, String)> = db.query_row(
                    "select source_conversation_id,context_snapshot_id from agent_fork_turn_sources where conversation_id=?1 and turn_id=?2",
                    params![conversation.to_string(), turn.to_string()], |row| Ok((row.get(0)?, row.get(1)?))
                ).optional().map_err(db_error)?;
    let (origin, snapshot) = source.ok_or(SubmissionError::NotFound)?;
    let origin = Uuid::parse_str(&origin).map_err(|_| SubmissionError::Corrupt)?;
    if origin.is_nil() || origin == conversation {
        return Err(SubmissionError::Corrupt);
    }
    let record = read_turn(db, origin, turn)?.ok_or(SubmissionError::Corrupt)?;
    if record.context_snapshot_id.to_string() != snapshot {
        return Err(SubmissionError::Corrupt);
    }
    Ok(record)
}
fn read_turn(
    db: &Connection,
    conversation: Uuid,
    turn: Uuid,
) -> Result<Option<SubmissionRecord>, SubmissionError> {
    let request: Option<String> = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![conversation.to_string(), turn.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    request
        .map(|value| {
            let request = Uuid::parse_str(&value).map_err(|_| SubmissionError::Corrupt)?;
            read(db, conversation, request)?.ok_or(SubmissionError::Corrupt)
        })
        .transpose()
}
