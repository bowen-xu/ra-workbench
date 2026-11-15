//! Original lazy row decoding and shared Agent outcome reads. No Native wait.
#[cfg(test)]
mod tests;
use rusqlite::{Connection, OptionalExtension, types::Type};

use super::resources::Statements;
use crate::ai::error::AiError;
use research_workbench_native_sdk::analysis_history::{
    HistoryRequestV1,
    contracts::PaperAnalysisMessage,
    rows::{AttemptRowV1, CapturedTurnV1, CapturedWorkflowV1, TurnRowV1, WorkflowRowV1},
};

fn get_workflow(
    connection: &Connection,
    workflow_id: &str,
    sql: &Statements,
) -> Result<Option<CapturedWorkflowV1>, AiError> {
    let workflow = connection
        .query_row(sql.workflow, [workflow_id], |row| {
            Ok(WorkflowRowV1 {
                id: row.get(0)?,
                conversation_id: row.get::<_, Option<String>>(14)?,
                paper_id: row.get(1)?,
                context_snapshot_id: row.get(2)?,
                status: row.get(3)?,
                provider_id: row.get(4)?,
                model_id: row.get(5)?,
                reasoning_effort: row.get(6)?,
                prompt_set_version: row.get(7)?,
                active_turn_kind: row.get(8)?,
                revision: row.get(9)?,
                supersedes_workflow_id: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                completed_at: row.get(13)?,
            })
        })
        .optional()?;
    workflow
        .map(|row| {
            let turns = list_turns(connection, workflow_id, sql)?;
            Ok(CapturedWorkflowV1 { row, turns })
        })
        .transpose()
}

pub(crate) fn capture(
    connection: &Connection,
    request: &HistoryRequestV1,
    sql: &Statements,
) -> Result<Option<CapturedWorkflowV1>, AiError> {
    match request {
        HistoryRequestV1::Get { workflow_id } => get_workflow(connection, workflow_id, sql),
        HistoryRequestV1::Latest { paper_id } => {
            let id = connection
                .query_row(sql.latest, [paper_id], |row| row.get::<_, String>(0))
                .optional()?;
            match id {
                Some(id) => get_workflow(connection, &id, sql),
                None => Ok(None),
            }
        }
    }
}

fn list_turns(
    connection: &Connection,
    workflow_id: &str,
    sql: &Statements,
) -> Result<Vec<CapturedTurnV1>, AiError> {
    let mut statement = connection.prepare(sql.turns)?;
    let rows = statement.query_map([workflow_id], |row| {
        Ok(TurnRowV1 {
            id: row.get(0)?,
            workflow_id: row.get(1)?,
            ordinal: row.get(2)?,
            kind: row.get(3)?,
            status: row.get(4)?,
            candidate_id: row.get(5)?,
            created_at: row.get(6)?,
            finished_at: row.get(7)?,
        })
    })?;
    let mut turns = Vec::new();
    for row in rows {
        let row = row?;
        let attempts = list_attempts(connection, &row.id, sql)?;
        turns.push(CapturedTurnV1 { row, attempts });
    }
    Ok(turns)
}

fn list_attempts(
    connection: &Connection,
    turn_id: &str,
    sql: &Statements,
) -> Result<Vec<AttemptRowV1>, AiError> {
    let mut statement = connection.prepare(sql.attempts)?;
    let rows = statement.query_map([turn_id], |row| {
        let messages_json: String = row.get(4)?;
        let normalized_messages = serde_json::from_str::<Vec<PaperAnalysisMessage>>(&messages_json)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })?;
        Ok(AttemptRowV1 {
            id: row.get(0)?,
            turn_id: row.get(1)?,
            shared_conversation_id: row.get::<_, Option<String>>(22)?,
            shared_turn_outcome: None,
            shared_request_id: row.get::<_, Option<String>>(23)?,
            shared_turn_id: row.get::<_, Option<String>>(24)?,
            shared_ai_run_id: row.get(25)?,
            shared_candidate_id: row.get(26)?,
            shared_terminal_status: row.get(27)?,
            attempt: row.get(2)?,
            status: row.get(3)?,
            normalized_messages,
            provider_id: row.get(5)?,
            model_id: row.get(6)?,
            reasoning_effort: row.get(7)?,
            ai_run_id: row.get(8)?,
            candidate_id: row.get(9)?,
            input_hash: row.get(10)?,
            output_hash: row.get(11)?,
            input_tokens: row.get(12)?,
            output_tokens: row.get(13)?,
            error_code: row.get(14)?,
            error_message: row.get(15)?,
            created_at: row.get(16)?,
            started_at: row.get(17)?,
            finished_at: row.get(18)?,
            recovery_reason: row.get(19)?,
            recovery_version: row.get(20)?,
            recovered_at: row.get(21)?,
        })
    })?;
    let mut attempts = rows.collect::<Result<Vec<_>, _>>()?;
    for attempt in &mut attempts {
        let Some(conversation_id) = attempt
            .shared_conversation_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
        else {
            continue;
        };
        let Some(turn_id) = attempt
            .shared_turn_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
        else {
            continue;
        };
        let outcome = crate::agent::repository::projection::read_turn_outcome(
            connection,
            research_agent_core::conversation::ConversationId::from_uuid(conversation_id),
            turn_id,
        )?;
        attempt.shared_turn_outcome = outcome.map(|value| {
            match value {
                research_agent_core::protocol::TurnOutcome::Completed => "completed",
                research_agent_core::protocol::TurnOutcome::Failed => "failed",
                research_agent_core::protocol::TurnOutcome::Canceled => "canceled",
                research_agent_core::protocol::TurnOutcome::Interrupted => "interrupted",
            }
            .to_owned()
        });
    }
    Ok(attempts)
}
