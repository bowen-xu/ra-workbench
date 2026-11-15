//! Read/write measurements through the existing SQLite executor and journal fences.
use crate::agent::context_usage::*;
use research_agent_core::protocol::ModelIdentity;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

#[cfg(test)]
#[path = "context_usage/tests.rs"]
mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("context usage input is invalid")]
    Invalid,
    #[error("context usage conversation is unavailable")]
    NotFound,
    #[error("context usage window changed")]
    Conflict,
    #[error("context usage observation is corrupt")]
    Corrupt,
    #[error("context usage storage failed")]
    Storage(#[from] rusqlite::Error),
}

fn identity(db: &Connection, conversation: Uuid) -> Result<(u64, Option<Uuid>, u64), Error> {
    let row: Option<(bool, u64)> = db
        .query_row(
            "select archived,durable_count from agent_conversations where id=?1",
            [conversation.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((false, watermark)) = row else {
        return Err(Error::NotFound);
    };
    let window: Option<String> = db
        .query_row(
            "select window_id from agent_active_context_windows where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let window = window
        .map(|value| Uuid::parse_str(&value).map_err(|_| Error::Corrupt))
        .transpose()?;
    let epoch = db.query_row(
        "select count(*) from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='conversationRolledBack'",
        [conversation.to_string()], |row| row.get(0),
    )?;
    Ok((watermark, window, epoch))
}

pub(crate) fn record(
    db: &mut Connection,
    conversation: Uuid,
    sample: &WindowUsageSample,
) -> Result<(), Error> {
    if conversation.is_nil() || !sample.validate() {
        return Err(Error::Invalid);
    }
    let model = serde_json::to_string(&sample.model).map_err(|_| Error::Invalid)?;
    let encoded = serde_json::to_string(sample).map_err(|_| Error::Invalid)?;
    if model.len() > 4096 || encoded.len() > 8192 {
        return Err(Error::Invalid);
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (watermark, window, epoch) = identity(&tx, conversation)?;
    if watermark == 0 || window != sample.window_id {
        return Err(Error::Conflict);
    }
    // A compaction candidate cannot manufacture a sampling observation: only
    // the native response callback calls this port, after the journal finish.
    let finish: Option<(u64, Option<u64>, Option<u64>)> = tx.query_row(
        "select ordinal,json_extract(payload_json,'$.payload.inputTokens'),json_extract(payload_json,'$.payload.outputTokens') from agent_items
         where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='modelRequestFinished'
         and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.attemptId')=?3",
        params![conversation.to_string(),sample.turn_id.to_string(),sample.request_id.to_string()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    let Some((finish_ordinal, input, output)) = finish else {
        return Err(Error::Conflict);
    };
    if (input, output) != (sample.input_tokens, sample.output_tokens) {
        return Err(Error::Conflict);
    }
    let advanced: bool = tx.query_row(
        "select exists(select 1 from agent_items where conversation_id=?1 and ordinal>?2 and json_extract(payload_json,'$.payload.type') not in ('phaseChanged','turnFinished','usage','itemPhaseChanged','itemCompleted'))",
        params![conversation.to_string(),finish_ordinal], |row| row.get(0),
    )?;
    if advanced {
        return Err(Error::Conflict);
    }
    let window = window.map(|value| value.to_string());
    tx.execute(
        "insert into agent_context_usage_observations(conversation_id,model_json,window_id,window_key,rollback_epoch,source_ordinal,sample_json)
         values(?1,?2,?3,coalesce(?3,''),?4,?5,?6)
         on conflict(conversation_id,model_json,window_key,rollback_epoch) do update set
         source_ordinal=excluded.source_ordinal,sample_json=excluded.sample_json,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         where excluded.source_ordinal>=agent_context_usage_observations.source_ordinal",
        params![conversation.to_string(),model,window,epoch,watermark-1,encoded],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn read(
    db: &mut Connection,
    conversation: Uuid,
    model: ModelIdentity,
) -> Result<WindowUsageResponse, Error> {
    if conversation.is_nil()
        || model.provider_id.trim().is_empty()
        || model.model_id.trim().is_empty()
    {
        return Err(Error::Invalid);
    }
    let model_json = serde_json::to_string(&model).map_err(|_| Error::Invalid)?;
    if model_json.len() > 4096 {
        return Err(Error::Invalid);
    }
    let tx = db.transaction()?;
    let (watermark, window_id, rollback_epoch) = identity(&tx, conversation)?;
    let saved: Option<(u64,String)> = tx.query_row(
        "select source_ordinal,sample_json from agent_context_usage_observations where conversation_id=?1 and model_json=?2 and window_key=?3 and rollback_epoch=?4",
        params![conversation.to_string(),model_json,window_id.map(|id| id.to_string()).unwrap_or_default(),rollback_epoch],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let mut response = WindowUsageResponse {
        schema_version: 1,
        conversation_id: conversation,
        model,
        window_id,
        watermark,
        rollback_epoch,
        status: WindowUsageStatus::Unknown,
        current_tokens: None,
        observation: None,
    };
    if let Some((ordinal, encoded)) = saved {
        let sample: WindowUsageSample =
            serde_json::from_str(&encoded).map_err(|_| Error::Corrupt)?;
        if !sample.validate()
            || sample.model != response.model
            || sample.window_id != window_id
            || ordinal >= watermark
        {
            return Err(Error::Corrupt);
        }
        let stale: bool = tx.query_row(
            "select exists(select 1 from agent_items where conversation_id=?1 and ordinal>?2 and json_extract(payload_json,'$.payload.type') not in ('phaseChanged','turnFinished','usage'))",
            params![conversation.to_string(),ordinal], |row| row.get(0),
        )?;
        response.status = if stale {
            WindowUsageStatus::Stale
        } else if sample.observed_tokens().is_some() {
            WindowUsageStatus::Observed
        } else if sample.estimated_tokens.is_some() {
            WindowUsageStatus::Estimated
        } else {
            WindowUsageStatus::Unknown
        };
        if !stale {
            response.current_tokens = sample.observed_tokens().or(sample.estimated_tokens);
        }
        response.observation = Some(WindowUsageObservation {
            source_watermark: ordinal + 1,
            rollback_epoch,
            sample,
        });
    }
    tx.commit()?;
    Ok(response)
}
