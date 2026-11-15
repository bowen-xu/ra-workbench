//! Original run, shared submission and scope writes inside the caller-owned transaction.
use super::*;
use crate::ai::model::AiOverviewRetryContext;

pub(super) fn insert_run_records(
    source: impl FnOnce(&Transaction<'_>, &str, Uuid) -> Result<Uuid, AiError>,
    transaction: &Transaction<'_>,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
    identities: &RunCreateIdentities,
    preview: &AiTaskPreview,
    retry_context: Option<&AiOverviewRetryContext>,
) -> Result<AiRun, AiError> {
    let run_id = identities.run.to_string();
    insert_run_record(transaction, &run_id, draft, preview)?;
    let has_legacy_bindings: bool = transaction.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy_bindings && uses_shared_agent_compatibility(&draft.task_kind) {
        let run_id = Uuid::parse_str(&run_id).map_err(|_| AiError::TemporaryUnavailable)?;
        if let Some((conversation_id, request_id)) = shared_submission {
            let exists = transaction.query_row(
                "select exists(select 1 from agent_submissions
                 where conversation_id = ?1 and request_id = ?2)",
                params![conversation_id.to_string(), request_id.to_string()],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(AiError::InvalidInput(
                    "结构化阅读共享提交身份不存在".to_owned(),
                ));
            }
            transaction.execute(
                "insert into agent_legacy_ai_run_bindings
                 (ai_run_id, conversation_id, request_id, task_kind)
                 values (?1, ?2, ?3, ?4)",
                params![
                    run_id.to_string(),
                    conversation_id.to_string(),
                    request_id.to_string(),
                    draft.task_kind
                ],
            )?;
        } else if draft.task_kind == "paper_profile_classification" {
            let paper_id = preview
                .scope_items
                .first()
                .and_then(|item| item.parent_object_id.as_deref())
                .ok_or_else(|| AiError::InvalidInput("画像任务缺少所属文献".into()))?;
            crate::ai::portrait_conversations::insert_binding(
                source,
                transaction,
                run_id,
                paper_id,
                identities.conversation_candidate,
                identities.request,
            )?;
        } else {
            crate::agent::repository::legacy_ai_runs::insert_in_with_ids(
                transaction,
                run_id,
                &draft.task_kind,
                identities.conversation_candidate,
                identities.request,
            )?;
        }
    }
    for item in &preview.scope_items {
        transaction.execute(
            "insert into ai_scope_items
             (id, run_id, ordinal, item_kind, object_id, parent_object_id, content_sha256,
              object_revision, snapshot_hash, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                item.id,
                run_id,
                item.ordinal,
                item.item_kind,
                item.object_id,
                item.parent_object_id,
                item.content_sha256,
                item.object_revision,
                item.snapshot_hash,
            ],
        )?;
    }
    if let Some(context) = retry_context {
        insert_retry_context(transaction, &run_id, context)?;
    }
    if let Some(context) = preview.overview_profile.as_ref() {
        insert_overview_profile(transaction, &run_id, context)?;
    }
    if let Some(candidate_kind) = preview.suggestion_candidate_kind.as_deref() {
        insert_suggestion_context(transaction, &run_id, candidate_kind)?;
    }
    if let Some(context) = preview.matrix_synthesis_context.as_ref() {
        insert_matrix_synthesis_context(transaction, &run_id, context)?;
    }
    // Read the returned projection before commit so a lookup failure cannot
    // turn an already-created run into an apparently failed creation.
    get_run(transaction, &run_id)
}

fn insert_run_record(
    transaction: &Transaction<'_>,
    run_id: &str,
    draft: &AiTaskSpecDraft,
    preview: &AiTaskPreview,
) -> Result<(), AiError> {
    transaction.execute(
        "insert into ai_runs
         (id, task_kind, status, provider_id, model_id, reasoning_effort, pipeline_version, prompt_version,
          output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
          max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
          created_at, updated_at)
         values (?1, ?2, 'queued', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, case when ?15 is null then null else strftime('%Y-%m-%dT%H:%M:%fZ', 'now') end,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            run_id,
            draft.task_kind,
            preview.provider_id,
            preview.model_id,
            preview.reasoning_effort,
            draft.pipeline_version,
            draft.prompt_version,
            draft.output_schema_version,
            draft.user_goal.trim(),
            preview.scope_hash,
            preview.input_item_count,
            draft.max_input_tokens,
            draft.max_output_tokens,
            draft.timeout_ms,
            draft.cloud_consent_version,
        ],
    )?;
    Ok(())
}
