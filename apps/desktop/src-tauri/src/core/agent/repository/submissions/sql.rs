//! Shared repository implementation; owned writes retain the actual SQL error.
use super::*;
use crate::agent::repository::SqlRepositoryError as E;
type Result<T> = std::result::Result<T, E<SubmissionError>>;

pub(crate) fn insert_in_sql(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    input: SubmissionInput,
) -> Result<SubmissionRecord> {
    input.validate().map_err(E::Domain)?;
    insert_validated_sql(tx, conversation, request, input)
}
pub(super) fn ensure_no_child_cleanup(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
) -> Result<()> {
    let (has_table, migrated): (bool, bool) = tx.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_child_cleanup_intents'), exists(select 1 from schema_migrations where version=219)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !has_table {
        return if migrated {
            Err(E::Domain(SubmissionError::Unavailable))
        } else {
            // A database predating the cleanup-intent migration cannot have
            // admitted a cleanup intent through that owner.
            Ok(())
        };
    }
    let cleanup_pending: bool = tx.query_row(
        "select exists(select 1 from agent_child_cleanup_intents where child_id=?1)",
        [conversation.to_string()],
        |row| row.get(0),
    )?;
    if cleanup_pending {
        return Err(E::Domain(SubmissionError::Conflict));
    }
    Ok(())
}

pub(super) fn insert_validated_sql(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    input: SubmissionInput,
) -> Result<SubmissionRecord> {
    ensure_no_child_cleanup(tx, conversation)?;
    if let Some(record) = read_sql(tx, conversation, request)? {
        return if record.input == input {
            Ok(record)
        } else {
            Err(E::Domain(SubmissionError::Conflict))
        };
    }
    let exists: bool = tx.query_row(
        "select exists(select 1 from agent_conversations where id=?1)",
        [conversation.to_string()],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(E::Domain(SubmissionError::NotFound));
    }
    let turn_id = Uuid::new_v4();
    let context_snapshot_id = Uuid::new_v4();
    let json =
        serde_json::to_string(&input).map_err(|_| E::Domain(SubmissionError::InvalidInput))?;
    tx.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,?5)",
        params![conversation.to_string(),request.to_string(),turn_id.to_string(),context_snapshot_id.to_string(),json])?;
    Ok(SubmissionRecord {
        conversation_id: conversation,
        request_id: request,
        turn_id,
        context_snapshot_id,
        input,
    })
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObservedSubmission {
    pub record: SubmissionRecord,
    pub input_json: String,
}
pub(crate) fn read_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<SubmissionRecord>> {
    read_sql_observed(db, conversation, request).map(|observed| observed.map(|value| value.record))
}
pub(crate) fn read_sql_observed(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<ObservedSubmission>> {
    let raw:Option<(String,String,i64,String)>=db.query_row(
        "select turn_id,context_snapshot_id,schema_version,input_json from agent_submissions where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(),request.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    raw.map(|(turn, snapshot, version, json)| {
        if version != 1 {
            return Err(E::Domain(SubmissionError::Corrupt));
        }
        let input: SubmissionInput =
            serde_json::from_str(&json).map_err(|_| E::Domain(SubmissionError::Corrupt))?;
        let images = if input.text.trim().is_empty() {
            read_images_sql(db, conversation, request)?
        } else {
            vec![]
        };
        input
            .validate_with_image_references(&images)
            .map_err(|_| E::Domain(SubmissionError::Corrupt))?;
        Ok(ObservedSubmission {
            record: SubmissionRecord {
                conversation_id: conversation,
                request_id: request,
                turn_id: Uuid::parse_str(&turn).map_err(|_| E::Domain(SubmissionError::Corrupt))?,
                context_snapshot_id: Uuid::parse_str(&snapshot)
                    .map_err(|_| E::Domain(SubmissionError::Corrupt))?,
                input,
            },
            input_json: json,
        })
    })
    .transpose()
}
pub(super) fn read_images_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Vec<SubmissionImageReference>> {
    let mut statement=db.prepare("select source_id,content_sha256,media_type,byte_size,ordinal from agent_submission_images where conversation_id=?1 and request_id=?2 order by ordinal")?;
    let rows = statement.query_map(
        params![conversation.to_string(), request.to_string()],
        |row| {
            Ok(SubmissionImageReference {
                source_id: row.get(0)?,
                content_sha256: row.get(1)?,
                media_type: row.get(2)?,
                byte_size: row.get(3)?,
                ordinal: row.get(4)?,
            })
        },
    )?;
    let refs = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    validate_image_references(&refs).map_err(|_| E::Domain(SubmissionError::Corrupt))?;
    Ok(refs)
}
