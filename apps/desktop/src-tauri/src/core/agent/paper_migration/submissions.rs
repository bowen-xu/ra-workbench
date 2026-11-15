//! Frozen legacy context and images use the original submission materializer.
use super::events::{legacy_context_snapshot_id, legacy_request_id, legacy_turn_id};
use super::{AppStore, Error, LegacySession, LegacyTurn};
use crate::ai::paper_context::agent_preview;
use research_agent_core::{
    protocol::ModelIdentity,
    submission::{SubmissionImage, SubmissionInput},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::HashSet;
use uuid::Uuid;

pub(super) fn prepare_legacy_submissions(
    store: &AppStore,
    paper_id: &str,
    legacy: &LegacySession,
    turns: &[LegacyTurn],
    conversation_id: Uuid,
) -> Result<(), Error> {
    let Some(snapshot_id) = legacy.context_snapshot_id.as_deref() else {
        return Ok(());
    };
    let store_for_images = store.clone();
    let mut db = store.connection().map_err(|_| Error::Unavailable)?;
    (store
        .require_paper_reading_source()
        .map_err(|_| Error::Unavailable)?
        .snapshot_hash)(&db, snapshot_id, paper_id)
    .map_err(|_| Error::Unavailable)?
    .ok_or(Error::Corrupt)?;
    let (snapshot_id, context_hash) = agent_preview::recanonicalize_snapshot(
        store
            .require_paper_reading_source()
            .map_err(|_| Error::Unavailable)?,
        &mut db,
        snapshot_id,
    )
    .map_err(|_| Error::Corrupt)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Unavailable)?;
    for turn in turns {
        if turn.user_message.trim().is_empty() {
            continue;
        }
        let item_ids = legacy_context_item_ids(&tx, &turn.id)?;
        let context = if item_ids.is_empty() {
            Vec::new()
        } else {
            agent_preview::materialize_submission_context_for_items(
                store
                    .require_paper_reading_source()
                    .map_err(|_| Error::Unavailable)?,
                &tx,
                paper_id,
                &snapshot_id,
                &context_hash,
                &item_ids,
            )
            .map_err(|_| Error::Corrupt)?
        };
        let images = if item_ids.is_empty() {
            Vec::new()
        } else {
            agent_preview::materialize_submission_images_for_items(
                store
                    .require_paper_reading_source()
                    .map_err(|_| Error::Unavailable)?,
                &tx,
                &store_for_images,
                paper_id,
                &snapshot_id,
                &context_hash,
                Some(&item_ids),
            )
            .map_err(|_| Error::Corrupt)?
        };
        let input = SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            text: turn.user_message.clone(),
            model: ModelIdentity {
                provider_id: if turn.provider_id.trim().is_empty() {
                    "legacy".into()
                } else {
                    turn.provider_id.clone()
                },
                model_id: if turn.model_id.trim().is_empty() {
                    "legacy".into()
                } else {
                    turn.model_id.clone()
                },
                revision: None,
            },
            reasoning_effort: turn.reasoning_effort.clone(),
            service_tier: None,
            context,
            instructions: Vec::new(),
            instruction_configs: Vec::new(),
            skills: Vec::new(),
        };
        input.validate().map_err(|_| Error::Corrupt)?;
        let request_id = legacy_request_id(&legacy.id, &turn.id);
        let turn_id = legacy_turn_id(&legacy.id, &turn.id);
        let json = serde_json::to_string(&input).map_err(|_| Error::Corrupt)?;
        let existing: Option<(String, String, String)> = tx
            .query_row(
                "select turn_id,context_snapshot_id,input_json from agent_submissions where conversation_id=?1 and request_id=?2",
                params![conversation_id.to_string(), request_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| Error::Unavailable)?;
        let is_new = existing.is_none();
        if let Some((stored_turn, stored_snapshot, stored_json)) = existing {
            if stored_turn != turn_id.to_string()
                || stored_snapshot != legacy_context_snapshot_id(&legacy.id, &turn.id).to_string()
                || stored_json != json
            {
                return Err(Error::Conflict);
            }
        } else {
            tx.execute(
                "insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,?5)",
                params![conversation_id.to_string(), request_id.to_string(), turn_id.to_string(), legacy_context_snapshot_id(&legacy.id, &turn.id).to_string(), json],
            )
            .map_err(|_| Error::Unavailable)?;
        }
        let scope: Option<(String, String, String, String)> = tx
            .query_row(
                "select turn_id,paper_id,paper_context_snapshot_id,context_hash from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
                params![conversation_id.to_string(), request_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|_| Error::Unavailable)?;
        if scope.as_ref().is_some_and(|stored| {
            stored
                != &(
                    turn_id.to_string(),
                    paper_id.to_owned(),
                    snapshot_id.to_owned(),
                    context_hash.clone(),
                )
        }) {
            return Err(Error::Conflict);
        }
        if scope.is_none() {
            tx.execute(
                "insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,?4,?5,?6)",
                params![conversation_id.to_string(), request_id.to_string(), turn_id.to_string(), paper_id, snapshot_id, context_hash],
            )
            .map_err(|_| Error::Unavailable)?;
        }
        let stored_images = {
            let mut query = tx.prepare("select source_id,content_sha256,media_type,byte_size,ordinal from agent_submission_images where conversation_id=?1 and request_id=?2 order by ordinal").map_err(|_| Error::Unavailable)?;
            let rows = query
                .query_map(
                    params![conversation_id.to_string(), request_id.to_string()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .map_err(|_| Error::Unavailable)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::Unavailable)?
        };
        if is_new {
            if !stored_images.is_empty() {
                return Err(Error::Conflict);
            }
            insert_legacy_images(&tx, conversation_id, request_id, &images)?;
        } else if stored_images.len() != images.len()
            || stored_images
                .iter()
                .zip(&images)
                .enumerate()
                .any(|(ordinal, (stored, image))| {
                    stored.0 != image.source_id
                        || stored.1 != image.sha256
                        || stored.2 != image.media_type
                        || usize::try_from(stored.3).ok() != Some(image.bytes.len())
                        || usize::try_from(stored.4).ok() != Some(ordinal)
                })
        {
            return Err(Error::Conflict);
        }
    }
    tx.commit().map_err(|_| Error::Unavailable)
}

fn legacy_context_item_ids(
    db: &rusqlite::Connection,
    turn_id: &str,
) -> Result<HashSet<String>, Error> {
    let mut statement = db
        .prepare("select context_item_id from paper_ai_reading_turn_context_items where turn_id=?1")
        .map_err(|_| Error::Unavailable)?;
    let rows = statement
        .query_map([turn_id], |row| row.get::<_, String>(0))
        .map_err(|_| Error::Unavailable)?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|_| Error::Unavailable)
}

fn insert_legacy_images(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    request_id: Uuid,
    images: &[SubmissionImage],
) -> Result<(), Error> {
    for (ordinal, image) in images.iter().enumerate() {
        tx.execute(
            "insert into agent_submission_images(conversation_id,request_id,source_id,content_sha256,media_type,byte_size,ordinal) values(?1,?2,?3,?4,?5,?6,?7)",
            params![conversation_id.to_string(), request_id.to_string(), image.source_id, image.sha256, image.media_type, i64::try_from(image.bytes.len()).map_err(|_| Error::Corrupt)?, ordinal as i64],
        )
        .map_err(|_| Error::Unavailable)?;
    }
    Ok(())
}
