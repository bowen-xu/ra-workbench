//! Durable input admission uses the same connection as the conversation journal.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::submission::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use uuid::Uuid;
#[path = "submissions/history.rs"]
mod history;
pub(super) use history::read_historical_record;
#[path = "submissions/images.rs"]
mod images;
#[path = "submissions/sql.rs"]
mod sql;
#[path = "submissions/start_observation.rs"]
pub(crate) mod start_observation;
pub(crate) use images::insert_in_with_images;
pub(crate) use sql::{ObservedSubmission, insert_in_sql, read_sql, read_sql_observed};

pub struct SqliteSubmissionStore {
    connection: SharedDatabase,
    evidence_store: Option<crate::storage::AppStore>,
}
impl SqliteSubmissionStore {
    pub(crate) fn new(connection: impl IntoDatabase) -> Self {
        Self {
            connection: connection.into_database(),
            evidence_store: None,
        }
    }
    pub fn for_app(store: &crate::storage::AppStore) -> Self {
        Self {
            connection: store.shared_agent_connection(),
            evidence_store: Some(store.clone()),
        }
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, SubmissionError> + Send + 'static,
    ) -> Result<T, SubmissionError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = connection
                .lock()
                .map_err(|_| SubmissionError::Unavailable)?;
            operation(&mut db)
        })
        .await
        .map_err(|_| SubmissionError::Unavailable)?
    }

    pub async fn latest_for_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<SubmissionRecord, SubmissionError> {
        self.run(move |db| {
            let request: String = db
                .query_row(
                    "select request_id from agent_submissions where conversation_id=?1 order by rowid desc limit 1",
                    [conversation_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?
                .ok_or(SubmissionError::NotFound)?;
            let request = Uuid::parse_str(&request).map_err(|_| SubmissionError::Corrupt)?;
            read(db, conversation_id, request)?.ok_or(SubmissionError::NotFound)
        })
        .await
    }
}
impl SubmissionStore for SqliteSubmissionStore {
    fn evidence_content<'a>(
        &'a self,
        input: &'a [research_agent_core::user_input::UserInput],
    ) -> SubmissionFuture<'a, Vec<research_agent_core::model::ContentPart>> {
        let input = input.to_vec();
        let store = self.evidence_store.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                research_agent_core::user_input::evidence::content(&input, |image| {
                    crate::agent::evidence_images::read(
                        store.as_ref().ok_or(SubmissionError::Unavailable)?,
                        image,
                    )
                })
            })
            .await
            .map_err(|_| SubmissionError::Unavailable)?
        })
    }
    fn get_image_references(
        &self,
        conversation: Uuid,
        request: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImageReference>> {
        Box::pin(self.run(move |db| {
            read(db, conversation, request)?.ok_or(SubmissionError::NotFound)?;
            read_image_manifest(db, conversation, request)
        }))
    }
    fn get_history_steering(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> SubmissionFuture<'_, Vec<research_agent_core::steering::SteeringInput>> {
        Box::pin(self.run(move |db| {
            let record = history::read_historical_record(db, conversation, turn)?;
            let mut statement = db.prepare("select input_id,text from agent_steering_inputs where turn_id=?1 order by ordinal").map_err(db_error)?;
            let rows = statement.query_map([turn.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(db_error)?;
            rows.map(|row| {
                let (id, text) = row.map_err(db_error)?;
                let input_id = Uuid::parse_str(&id).map_err(|_| SubmissionError::Corrupt)?;
                let (user_input, skill_instructions) = super::steering::resources::read(db, input_id).map_err(|_| SubmissionError::Corrupt)?;
                let input = research_agent_core::steering::SteeringInput { scope: research_agent_core::steering::SteeringScope {conversation_id: record.conversation_id, turn_id: turn}, input_id, text, user_input, skill_instructions };
                input.validate().map_err(|_| SubmissionError::Corrupt)?;
                Ok(input)
            }).collect()
        }))
    }
    fn get_history_input(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
    ) -> SubmissionFuture<'_, HistoricalTurnInput> {
        Box::pin(self.historical_input(conversation_id, turn_id))
    }
    fn get_by_turn(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
    ) -> SubmissionFuture<'_, SubmissionRecord> {
        Box::pin(async move {
            self.run(move |db| {
                let request: String = db.query_row("select request_id from agent_submissions where conversation_id = ?1 and turn_id = ?2", params![conversation_id.to_string(), turn_id.to_string()], |row| row.get(0)).optional().map_err(db_error)?.ok_or(SubmissionError::NotFound)?;
                let request = Uuid::parse_str(&request).map_err(|_| SubmissionError::Corrupt)?;
                read(db, conversation_id, request)?.ok_or(SubmissionError::NotFound)
            }).await
        })
    }
    fn put(
        &self,
        conversation_id: Uuid,
        request_id: Uuid,
        input: SubmissionInput,
    ) -> SubmissionFuture<'_, SubmissionRecord> {
        Box::pin(async move {
            input.validate()?;
            self.run(move |db| {
                let tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                let record = insert_in(&tx, conversation_id, request_id, input)?;
                tx.commit().map_err(db_error)?;
                Ok(record)
            })
            .await
        })
    }
    fn get(
        &self,
        conversation_id: Uuid,
        request_id: Uuid,
    ) -> SubmissionFuture<'_, SubmissionRecord> {
        Box::pin(async move {
            self.run(move |db| {
                read(db, conversation_id, request_id)?.ok_or(SubmissionError::NotFound)
            })
            .await
        })
    }
}
/// Caller owns the transaction so scope checks and admission are atomic.
pub(crate) fn insert_in(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    request_id: Uuid,
    input: SubmissionInput,
) -> Result<SubmissionRecord, SubmissionError> {
    input.validate()?;
    insert_validated(tx, conversation_id, request_id, input)
}

fn insert_validated(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    request_id: Uuid,
    input: SubmissionInput,
) -> Result<SubmissionRecord, SubmissionError> {
    sql::insert_validated_sql(tx, conversation_id, request_id, input)
        .map_err(|cause| cause.domain_or(SubmissionError::Unavailable))
}

/// Create a fresh turn from an already frozen submission. This is the durable
/// retry primitive used after an interrupted child turn: the old turn remains
/// immutable in history, while the new request receives new turn/context
/// identities and the exact same validated input and image manifest.
pub(crate) fn retry_frozen_in(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    source_request_id: Uuid,
    retry_request_id: Uuid,
) -> Result<SubmissionRecord, SubmissionError> {
    if conversation_id.is_nil()
        || source_request_id.is_nil()
        || retry_request_id.is_nil()
        || source_request_id == retry_request_id
    {
        return Err(SubmissionError::InvalidInput);
    }
    let source = read(tx, conversation_id, source_request_id)?.ok_or(SubmissionError::NotFound)?;
    let images = read_image_manifest(tx, conversation_id, source_request_id)?;
    source.input.validate_with_image_references(&images)?;
    let retry = insert_validated(tx, conversation_id, retry_request_id, source.input)?;
    tx.execute(
        "insert into agent_submission_images(
            conversation_id,request_id,source_id,content_sha256,media_type,byte_size,ordinal
         ) select conversation_id,?1,source_id,content_sha256,media_type,byte_size,ordinal
           from agent_submission_images
          where conversation_id=?2 and request_id=?3",
        params![
            retry_request_id.to_string(),
            conversation_id.to_string(),
            source_request_id.to_string()
        ],
    )
    .map_err(db_error)?;
    Ok(retry)
}

pub(crate) fn read(
    db: &Connection,
    conversation_id: Uuid,
    request_id: Uuid,
) -> Result<Option<SubmissionRecord>, SubmissionError> {
    read_sql(db, conversation_id, request_id)
        .map_err(|cause| cause.domain_or(SubmissionError::Unavailable))
}
fn db_error(_: rusqlite::Error) -> SubmissionError {
    SubmissionError::Unavailable
}

#[cfg(test)]
#[path = "submissions/tests.rs"]
mod tests;

/// Persist validated image references beside a submission. Bytes stay in the
/// content-addressed store; this table only binds immutable hashes to input.
pub(crate) fn insert_image_manifest(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    request_id: Uuid,
    images: &[research_agent_core::submission::SubmissionImage],
) -> Result<(), SubmissionError> {
    research_agent_core::submission::validate_submission_images(images)?;
    for (ordinal, image) in images.iter().enumerate() {
        if format!("{:x}", Sha256::digest(&image.bytes)) != image.sha256 {
            return Err(SubmissionError::InvalidInput);
        }
        tx.execute("insert into agent_submission_images(conversation_id,request_id,source_id,content_sha256,media_type,byte_size,ordinal) values(?1,?2,?3,?4,?5,?6,?7)", params![conversation_id.to_string(),request_id.to_string(),image.source_id,image.sha256,image.media_type,i64::try_from(image.bytes.len()).map_err(|_| SubmissionError::InvalidInput)?,ordinal as i64]).map_err(db_error)?;
    }
    Ok(())
}

/// Read only the image metadata bound to this exact submission identity.
/// The execution adapter will consume this during multimodal admission.
#[allow(dead_code)]
pub(crate) fn read_image_manifest(
    db: &Connection,
    conversation_id: Uuid,
    request_id: Uuid,
) -> Result<Vec<SubmissionImageReference>, SubmissionError> {
    sql::read_images_sql(db, conversation_id, request_id)
        .map_err(|cause| cause.domain_or(SubmissionError::Unavailable))
}

/// Materialize persisted image references only from the canonical content root.
/// This is intentionally separate from metadata reads so callers must opt into
/// byte access during execution recovery.
#[allow(dead_code)]
pub(crate) fn materialize_image_manifest(
    root: &std::path::Path,
    refs: &[SubmissionImageReference],
) -> Result<Vec<research_agent_core::submission::SubmissionImage>, SubmissionError> {
    use crate::agent::instructions::directory::Directory;
    use std::io::Read;
    let root = root
        .canonicalize()
        .map_err(|_| SubmissionError::Unavailable)?;
    let mut result = Vec::with_capacity(refs.len());
    for reference in refs {
        if reference.media_type != "image/png" || reference.content_sha256.len() != 64 {
            return Err(SubmissionError::Corrupt);
        }
        let hash = &reference.content_sha256;
        let mut directory = Directory::root(&root).map_err(|_| SubmissionError::Unavailable)?;
        for part in ["ai-reading-images", &hash[..2], &hash[2..4]] {
            directory = directory
                .child(std::path::Path::new(part))
                .map_err(|_| SubmissionError::Corrupt)?;
        }
        let file = directory
            .config_file(&format!("{hash}.png"))
            .map_err(|_| SubmissionError::Unavailable)?
            .ok_or(SubmissionError::Corrupt)?;
        if file
            .metadata()
            .map_err(|_| SubmissionError::Unavailable)?
            .len()
            != reference.byte_size
        {
            return Err(SubmissionError::Corrupt);
        }
        let mut bytes = Vec::new();
        file.take(reference.byte_size + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SubmissionError::Unavailable)?;
        if format!("{:x}", Sha256::digest(&bytes)) != *hash
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            return Err(SubmissionError::Corrupt);
        }
        result.push(research_agent_core::submission::SubmissionImage {
            source_id: reference.source_id.clone(),
            media_type: reference.media_type.clone(),
            sha256: hash.clone(),
            bytes,
        });
    }
    Ok(result)
}

/// Native implementation of the Core image materialization port.
///
/// The writing host will construct this at the multimodal submission boundary;
/// keep the adapter compiled while that host input is still being migrated.
pub(crate) struct AppImageMaterializer {
    connection: SharedDatabase,
    store: crate::storage::AppStore,
}
impl AppImageMaterializer {
    pub(crate) fn new(store: &crate::storage::AppStore) -> Self {
        Self {
            connection: store.shared_agent_connection(),
            store: store.clone(),
        }
    }
    fn historical_images(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
        required_sha256: Option<Vec<String>>,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        let connection = Arc::clone(&self.connection);
        let store = self.store.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = connection
                    .lock()
                    .map_err(|_| SubmissionError::Unavailable)?;
                let tx = db.transaction().map_err(db_error)?;
                let record = history::read_historical_record(&tx, conversation_id, turn_id)?;
                let mut refs = read_image_manifest(&tx, record.conversation_id, record.request_id)?;
                if let Some(required) = required_sha256 {
                    refs.retain(|image| required.contains(&image.content_sha256));
                }
                tx.commit().map_err(db_error)?;
                drop(db);
                if refs.is_empty() {
                    return Ok(vec![]);
                }
                let _read = store
                    .content_read()
                    .map_err(|_| SubmissionError::Unavailable)?;
                materialize_image_manifest(store.content_directory(), &refs)
            })
            .await
            .map_err(|_| SubmissionError::Unavailable)?
        })
    }
}

impl ImageMaterializer for AppImageMaterializer {
    fn materialize_history(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        self.historical_images(conversation_id, turn_id, None)
    }

    fn materialize_history_images(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
        required_sha256: Vec<String>,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        self.historical_images(conversation_id, turn_id, Some(required_sha256))
    }

    fn materialize(
        &self,
        conversation_id: Uuid,
        request_id: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        let connection = Arc::clone(&self.connection);
        let store = self.store.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let db = connection
                    .lock()
                    .map_err(|_| SubmissionError::Unavailable)?;
                let refs = read_image_manifest(&db, conversation_id, request_id)?;
                drop(db);
                let _read = store
                    .content_read()
                    .map_err(|_| SubmissionError::Unavailable)?;
                materialize_image_manifest(store.content_directory(), &refs)
            })
            .await
            .map_err(|_| SubmissionError::Unavailable)?
        })
    }
}
