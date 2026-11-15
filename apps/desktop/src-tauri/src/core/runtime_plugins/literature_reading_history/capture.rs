use super::resources::ReadingHistoryResources;
use crate::{
    ai::error::AiError,
    runtime_plugins::literature_context::{ContextResult, source_failure, sql_failure},
};
use research_workbench_native_sdk::reading_history::{
    ReadingDebugRowV1, ReadingHistoryInputV1 as Input, ReadingHistoryRequestV1 as Request,
    contracts::{PaperAiReadingMessage, PaperAiReadingTurn},
};
use rusqlite::{Connection, OptionalExtension, types::Type};

pub(crate) fn capture(
    db: &Connection,
    resources: &ReadingHistoryResources,
    request: &Request,
) -> ContextResult<Input> {
    Ok(match request {
        Request::List { paper_id } => {
            let mut statement = db
                .prepare(&resources.session_list)
                .map_err(|error| sql_failure(error, "source-read"))?;
            let ids = statement
                .query_map([paper_id], |row| row.get::<_, String>(0))
                .map_err(|error| sql_failure(error, "source-read"))?;
            let mut rows = Vec::new();
            for id in ids {
                let row = resources
                    .preparation
                    .capture_session_summary(
                        db,
                        &id.map_err(|error| sql_failure(error, "source-read"))?,
                    )?
                    .ok_or_else(|| {
                        source_failure(AiError::InvalidInput("找不到 AI 阅读会话".into()))
                    })?;
                rows.push(row);
            }
            Input::List { rows }
        }
        Request::Session { session_id } => {
            let summary = resources
                .preparation
                .capture_session_summary(db, session_id)?;
            // The original missing summary rejects before reading any turn rows.
            let turns = if summary.is_some() {
                turns(db, &resources.session_turns, session_id).map_err(source_failure)?
            } else {
                Vec::new()
            };
            Input::Session {
                summary: summary.map(Box::new),
                turns,
            }
        }
        Request::Debug { turn_id } => {
            let row = db
                .query_row(&resources.turn_debug, [turn_id], |row| {
                    let raw_request_json = row.get(1)?;
                    let raw_response_json = row.get(2)?;
                    Ok(ReadingDebugRowV1 {
                        turn_id: row.get(0)?,
                        raw_request_json,
                        raw_response_json,
                    })
                })
                .optional()
                .map_err(|error| sql_failure(error, "source-read"))?;
            Input::DebugRecord { row }
        }
        Request::Archive { session_id } => Input::Archive {
            session_id: session_id.clone(),
        },
        Request::Bootstrap { .. } => return Err(source_failure(AiError::TemporaryUnavailable)),
    })
}
fn turns(db: &Connection, sql: &str, session: &str) -> Result<Vec<PaperAiReadingTurn>, AiError> {
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map([session], |row| {
        // Decode this original JSON column before every other column and next row.
        let messages_json: String = row.get(5)?;
        let messages = serde_json::from_str::<Vec<PaperAiReadingMessage>>(&messages_json).map_err(
            |error| rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(error)),
        )?;
        Ok(PaperAiReadingTurn {
            id: row.get(0)?,
            session_id: row.get(1)?,
            ordinal: row.get(2)?,
            user_message: row.get(3)?,
            assistant_message: row.get(4)?,
            messages,
            provider_id: row.get(6)?,
            model_id: row.get(7)?,
            reasoning_effort: row.get(8)?,
            context_window_tokens: row.get(9)?,
            context_window_source: row.get(10)?,
            status: row.get(11)?,
            input_hash: row.get(12)?,
            output_hash: row.get(13)?,
            input_tokens: row.get(14)?,
            output_tokens: row.get(15)?,
            error_code: row.get(16)?,
            error_message: row.get(17)?,
            concept_retrieval_enabled: row.get(18)?,
            concept_retrieval_trace_id: row.get(19)?,
            created_at: row.get(20)?,
            finished_at: row.get(21)?,
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}
pub(crate) fn archive(
    db: &Connection,
    resources: &ReadingHistoryResources,
    id: &str,
) -> ContextResult<bool> {
    db.execute(&resources.session_archive, [id])
        .map(|changed| changed > 0)
        .map_err(|error| sql_failure(error, "archive"))
}
pub(crate) fn snapshot_attachments(
    db: &Connection,
    resources: &ReadingHistoryResources,
    id: &str,
) -> ContextResult<Vec<(String, String, String)>> {
    let read = || -> rusqlite::Result<Vec<_>> {
        db.prepare(&resources.bootstrap_attachments)?
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
    };
    read().map_err(|error| sql_failure(error, "source-read"))
}
