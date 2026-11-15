#[path = "run_repository/preview.rs"]
mod preview;
pub(crate) use preview::{
    preview_task, resolve_task_scope, scope_hash, validate_fixed_task_scope,
    validate_task_before_policy,
};
use uuid::Uuid;
#[path = "run_repository/create.rs"]
mod create;
#[cfg(test)]
pub(crate) use create::insert_run;
pub(crate) use create::{
    RunCreateIdentities, insert_prepared_run_guarded, insert_run_guarded,
    uses_shared_agent_compatibility,
};

use research_agent_core::protocol::TurnOutcome;
use rusqlite::{Connection, OptionalExtension, params};

use super::{
    error::AiError,
    matrix_synthesis_context_repository::{
        insert_matrix_synthesis_context, load_matrix_synthesis_context,
    },
    model::{AiRun, AiRunStage, AiScopeItem},
    overview_profile_repository::{insert_overview_profile, load_overview_profile},
    overview_retry_repository::{
        insert_retry_context, load_retry_context, validate_retry_candidate,
    },
    suggestion_context_repository::{insert_suggestion_context, load_suggestion_context},
};

pub(crate) fn get_run(connection: &Connection, run_id: &str) -> Result<AiRun, AiError> {
    let mut run = connection
        .query_row(
            "select id, task_kind, status, provider_id, model_id, reasoning_effort, pipeline_version, prompt_version,
                output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
                max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
                cancel_requested, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens,
                error_code, error_message, created_at, started_at, finished_at, updated_at
         from ai_runs where id = ?1",
            [run_id],
            run_from_row,
        )
        .optional()?
        .ok_or_else(|| AiError::InvalidInput("找不到 AI 运行".to_owned()))?;
    run.scope_items = list_scope_items(connection, run_id)?;
    run.stages = list_stages(connection, run_id)?;
    run.overview_retry = load_retry_context(connection, run_id)?;
    run.overview_profile = load_overview_profile(connection, run_id)?;
    run.suggestion_candidate_kind = load_suggestion_context(connection, run_id)?;
    run.matrix_synthesis_context = load_matrix_synthesis_context(connection, run_id)?;
    let has_legacy_bindings: bool = connection.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy_bindings
        && let Some(binding) = crate::agent::repository::legacy_ai_runs::read(
            connection,
            Uuid::parse_str(run_id).map_err(|_| AiError::TemporaryUnavailable)?,
        )?
    {
        run.shared_conversation_id = Some(binding.conversation_id.as_uuid().to_string());
        run.shared_request_id = Some(binding.request_id.to_string());
        if let Some(turn_id) =
            shared_turn_id(connection, binding.conversation_id, binding.request_id)?
            && let Some(outcome) = crate::agent::repository::projection::read_turn_outcome(
                connection,
                binding.conversation_id,
                turn_id,
            )?
        {
            run.shared_turn_id = Some(turn_id.to_string());
            run.shared_turn_outcome = Some(shared_outcome_label(outcome).to_owned());
            // The shared journal is authoritative for migrated runs. Project
            // its terminal outcome into the legacy list response so a stale
            // ai_runs row cannot make history appear active forever after a
            // restart. This is read-only; the durable reconciliation still
            // happens through the legacy projection writer.
            if let Some(status) = crate::agent::repository::legacy_ai_runs::status_for_outcome(
                outcome,
                binding.candidate_id.is_some(),
                run.stages
                    .iter()
                    .any(|stage| stage.stage_kind == "validating" && stage.status == "failed"),
            ) {
                run.status = status.to_owned();
            }
        }
        run.shared_candidate_id = binding.candidate_id.map(|id| id.to_string());
        run.shared_terminal_status = binding.terminal_status;
    }
    Ok(run)
}

fn shared_outcome_label(outcome: TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Completed => "completed",
        TurnOutcome::Failed => "failed",
        TurnOutcome::Canceled => "canceled",
        TurnOutcome::Interrupted => "interrupted",
    }
}

#[cfg(test)]
#[path = "run_repository/shared_projection_tests.rs"]
mod shared_projection_tests;

pub(crate) fn list_runs(connection: &Connection, limit: i64) -> Result<Vec<AiRun>, AiError> {
    if !(1..=100).contains(&limit) {
        return Err(AiError::InvalidInput("运行列表数量无效".to_owned()));
    }
    let mut statement =
        connection.prepare("select id from ai_runs order by created_at desc limit ?1")?;
    let ids = statement
        .query_map([limit], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| get_run(connection, id)).collect()
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiRun> {
    Ok(AiRun {
        id: row.get(0)?,
        task_kind: row.get(1)?,
        status: row.get(2)?,
        provider_id: row.get(3)?,
        model_id: row.get(4)?,
        reasoning_effort: row.get(5)?,
        pipeline_version: row.get(6)?,
        prompt_version: row.get(7)?,
        output_schema_version: row.get(8)?,
        user_goal: row.get(9)?,
        scope_hash: row.get(10)?,
        input_item_count: row.get(11)?,
        max_input_tokens: row.get(12)?,
        max_output_tokens: row.get(13)?,
        timeout_ms: row.get(14)?,
        cloud_consent_version: row.get(15)?,
        cloud_consented_at: row.get(16)?,
        cancel_requested: row.get(17)?,
        input_tokens: row.get(18)?,
        output_tokens: row.get(19)?,
        cache_hit_tokens: row.get(20)?,
        cache_miss_tokens: row.get(21)?,
        error_code: row.get(22)?,
        error_message: row.get(23)?,
        created_at: row.get(24)?,
        started_at: row.get(25)?,
        finished_at: row.get(26)?,
        updated_at: row.get(27)?,
        scope_items: Vec::new(),
        stages: Vec::new(),
        overview_retry: None,
        overview_profile: None,
        suggestion_candidate_kind: None,
        matrix_synthesis_context: None,
        shared_conversation_id: None,
        shared_request_id: None,
        shared_turn_id: None,
        shared_turn_outcome: None,
        shared_candidate_id: None,
        shared_terminal_status: None,
    })
}

fn shared_turn_id(
    connection: &Connection,
    conversation_id: research_agent_core::conversation::ConversationId,
    request_id: Uuid,
) -> rusqlite::Result<Option<Uuid>> {
    connection
        .query_row(
            "select turn_id from agent_submissions where conversation_id=?1 and request_id=?2",
            params![
                conversation_id.as_uuid().to_string(),
                request_id.to_string()
            ],
            |row| {
                let value: String = row.get(0)?;
                Uuid::parse_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)
            },
        )
        .optional()
}

fn list_scope_items(connection: &Connection, run_id: &str) -> Result<Vec<AiScopeItem>, AiError> {
    let mut statement = connection.prepare("select id, ordinal, item_kind, object_id, parent_object_id, content_sha256, object_revision, snapshot_hash, created_at from ai_scope_items where run_id = ?1 order by ordinal")?;
    Ok(statement
        .query_map([run_id], |row| {
            Ok(AiScopeItem {
                id: row.get(0)?,
                ordinal: row.get(1)?,
                item_kind: row.get(2)?,
                object_id: row.get(3)?,
                parent_object_id: row.get(4)?,
                content_sha256: row.get(5)?,
                object_revision: row.get(6)?,
                snapshot_hash: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn list_stages(connection: &Connection, run_id: &str) -> Result<Vec<AiRunStage>, AiError> {
    let mut statement = connection.prepare("select id, run_id, stage_kind, attempt, status, input_hash, output_hash, input_tokens, output_tokens, duration_ms, error_code, error_message, started_at, finished_at, created_at from ai_run_stages where run_id = ?1 order by created_at, attempt")?;
    Ok(statement
        .query_map([run_id], |row| {
            Ok(AiRunStage {
                id: row.get(0)?,
                run_id: row.get(1)?,
                stage_kind: row.get(2)?,
                attempt: row.get(3)?,
                status: row.get(4)?,
                input_hash: row.get(5)?,
                output_hash: row.get(6)?,
                input_tokens: row.get(7)?,
                output_tokens: row.get(8)?,
                duration_ms: row.get(9)?,
                error_code: row.get(10)?,
                error_message: row.get(11)?,
                started_at: row.get(12)?,
                finished_at: row.get(13)?,
                created_at: row.get(14)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::uses_shared_agent_compatibility;

    #[test]
    fn shared_identity_covers_legacy_research_tasks_but_not_workflow_owned_turns() {
        for task_kind in [
            "selection_explanation",
            "reference_extraction",
            "matrix_synthesis",
            "paper_overview",
            "paper_suggestions",
        ] {
            assert!(uses_shared_agent_compatibility(task_kind), "{task_kind}");
        }
        for task_kind in ["paper_concept_map", "provider_diagnostic", "unknown"] {
            assert!(!uses_shared_agent_compatibility(task_kind), "{task_kind}");
        }
    }
}
