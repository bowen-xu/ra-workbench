//! Original task validation, model/scope reads and preview hashing in their original order.
use crate::ai::{
    credential::JsonCredentialStore,
    error::AiError,
    evidence_repository::preview_selection_evidence,
    llm_configuration,
    matrix_synthesis_evidence_repository::preview_matrix_synthesis_evidence,
    model::{AiScopeItem, AiScopeItemDraft, AiTaskPreview, AiTaskSpecDraft},
    overview_evidence_repository::preview_overview_evidence,
    overview_profile_repository::resolve_overview_profile,
    overview_retry_repository::{
        retry_evidence_preview, validate_retry_candidate, validate_retry_shape,
    },
    settings_repository::get_provider_settings,
};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;
#[derive(Serialize)]
struct ScopeIdentity<'a> {
    schema_version: u8,
    item_kind: &'a str,
    object_id: &'a str,
    parent_object_id: &'a Option<String>,
    content_sha256: &'a Option<String>,
    object_revision: &'a Option<i64>,
    source_version: &'a str,
}

#[derive(Serialize)]
struct TaskScopeFingerprint<'a> {
    schema_version: u8,
    task_kind: &'a str,
    item_snapshot_hashes: Vec<&'a str>,
    overview_retry: Option<&'a crate::ai::model::AiOverviewRetryContext>,
    overview_profile: Option<&'a crate::ai::model::AiOverviewProfileContext>,
    suggestion_candidate_kind: &'a Option<String>,
    matrix_synthesis_context: Option<&'a crate::ai::model::AiMatrixSynthesisContext>,
}

struct ResolvedScopeIdentity {
    parent_object_id: Option<String>,
    content_sha256: Option<String>,
    object_revision: Option<i64>,
    source_version: String,
}

fn resolved_identity(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResolvedScopeIdentity> {
    Ok(ResolvedScopeIdentity {
        parent_object_id: row.get(0)?,
        content_sha256: row.get(1)?,
        object_revision: row.get(2)?,
        source_version: row.get(3)?,
    })
}

pub(crate) fn preview_task(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    connection: &Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
) -> Result<AiTaskPreview, AiError> {
    validate_task(policy, draft)?;
    let scope = resolve_task_scope(connection, credentials, draft)?;
    let items = &scope.items;
    validate_task_scope(policy, &draft.task_kind, items)?;
    let retry = if draft.task_kind == "paper_overview" {
        validate_retry_candidate(policy, connection, draft, &items[0])?
    } else {
        None
    };
    let overview_profile = if draft.task_kind == "paper_overview" {
        resolve_overview_profile(
            policy,
            connection,
            draft,
            &items[0],
            retry.as_ref().map(|(context, _)| context),
        )?
    } else {
        None
    };
    let matrix_synthesis_context =
        (policy.matrix_context)(connection, draft.matrix_synthesis_context.as_ref(), items)?;
    let scope_hash = scope_hash(
        draft,
        items,
        retry.as_ref().map(|(context, _)| context),
        overview_profile.as_ref(),
        matrix_synthesis_context.as_ref(),
    )?;
    let evidence_preview = match (draft.task_kind.as_str(), retry.as_ref()) {
        ("selection_explanation", _) => {
            preview_selection_evidence(policy, connection, &items[0].object_id)?
        }
        ("paper_overview", Some((_, bundle))) => retry_evidence_preview(bundle),
        // Portrait scope previews describe file identity, never page-index excerpts.
        ("paper_profile_classification", _) => Vec::new(),
        ("paper_overview", None) => {
            preview_overview_evidence(policy, connection, &items[0].object_id)?
        }
        ("paper_concept_map", _) => {
            preview_overview_evidence(policy, connection, &items[0].object_id)?
        }
        ("paper_suggestions", _) => {
            preview_overview_evidence(policy, connection, &items[0].object_id)?
        }
        ("reference_extraction", _) => {
            preview_selection_evidence(policy, connection, &items[0].object_id)?
        }
        ("matrix_synthesis", _) => preview_matrix_synthesis_evidence(
            policy,
            connection,
            &matrix_synthesis_context
                .as_ref()
                .ok_or_else(|| AiError::InvalidInput("多文献综合缺少冻结上下文".to_owned()))?
                .evidence_anchor_ids,
        )?,
        _ => Vec::new(),
    };
    Ok(scope.into_preview(
        draft,
        scope_hash,
        overview_profile,
        matrix_synthesis_context,
        evidence_preview,
    ))
}

pub(crate) struct ResolvedTaskScope {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) is_cloud: bool,
    pub(crate) items: Vec<AiScopeItem>,
}

pub(crate) fn resolve_task_scope(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
) -> Result<ResolvedTaskScope, AiError> {
    let (provider_id, model_id, reasoning_effort, is_cloud) =
        if draft.model_selection.is_some() || draft.provider_id.trim().is_empty() {
            let selection = llm_configuration::resolve_model_selection(
                connection,
                &draft.task_kind,
                draft.model_selection.as_ref(),
            )?;
            let settings = llm_configuration::generation_settings(
                connection,
                credentials,
                &selection.provider_id,
                &selection.model_id,
            )?;
            (
                selection.provider_id,
                selection.model_id,
                selection.reasoning_effort,
                settings.is_cloud,
            )
        } else {
            let settings = get_provider_settings(connection, credentials, &draft.provider_id)?;
            (
                draft.provider_id.clone(),
                settings.model_id,
                None,
                settings.is_cloud,
            )
        };
    let mut seen = HashSet::new();
    let mut items = Vec::with_capacity(draft.scope_items.len());
    for (ordinal, requested) in draft.scope_items.iter().enumerate() {
        if !seen.insert((requested.item_kind.as_str(), requested.object_id.as_str())) {
            return Err(AiError::InvalidInput("AI 任务范围包含重复对象".to_owned()));
        }
        items.push(resolve_scope_item(connection, requested, ordinal as i64)?);
    }

    Ok(ResolvedTaskScope {
        provider_id,
        model_id,
        reasoning_effort,
        is_cloud,
        items,
    })
}

fn resolve_scope_item(
    connection: &Connection,
    requested: &AiScopeItemDraft,
    ordinal: i64,
) -> Result<AiScopeItem, AiError> {
    if requested.object_id.trim().is_empty() || requested.object_id.len() > 200 {
        return Err(AiError::InvalidInput("AI 范围对象 ID 无效".to_owned()));
    }
    let resolved = match requested.item_kind.as_str() {
        "paper" => connection
            .query_row(
                "select null, null, null, updated_at from bibliographic_items where id = ?1",
                [&requested.object_id],
                resolved_identity,
            )
            .optional()?,
        "attachment" => connection
            .query_row(
                "select a.item_id, a.content_sha256, null,
                        a.created_at || ':' || coalesce(j.updated_at, 'unindexed')
                 from attachments a left join full_text_index_jobs j on j.attachment_id = a.id
                 where a.id = ?1",
                [&requested.object_id],
                resolved_identity,
            )
            .optional()?,
        "note" => connection
            .query_row(
                "select page_id, null, revision, updated_at from note_blocks where id = ?1",
                [&requested.object_id],
                resolved_identity,
            )
            .optional()?,
        "anchor" => connection
            .query_row(
                "select attachment_id, content_sha256, selector_version, created_at
                 from source_anchors where id = ?1",
                [&requested.object_id],
                resolved_identity,
            )
            .optional()?,
        _ => return Err(AiError::InvalidInput("未知的 AI 范围对象类型".to_owned())),
    };
    let resolved =
        resolved.ok_or_else(|| AiError::InvalidInput("AI 任务范围中的对象不存在".to_owned()))?;
    let parent = resolved.parent_object_id;
    let sha256 = resolved.content_sha256;
    let revision = resolved.object_revision;
    if requested
        .parent_object_id
        .as_ref()
        .is_some_and(|value| Some(value) != parent.as_ref())
        || requested
            .content_sha256
            .as_ref()
            .is_some_and(|value| Some(value) != sha256.as_ref())
        || requested
            .object_revision
            .is_some_and(|value| Some(value) != revision)
    {
        return Err(AiError::InvalidInput(
            "AI 任务范围对象已变化，请重新预览".to_owned(),
        ));
    }
    let snapshot_hash = hash_json(&ScopeIdentity {
        schema_version: 1,
        item_kind: &requested.item_kind,
        object_id: &requested.object_id,
        parent_object_id: &parent,
        content_sha256: &sha256,
        object_revision: &revision,
        source_version: &resolved.source_version,
    })?;
    let created_at =
        connection.query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?;
    Ok(AiScopeItem {
        id: Uuid::new_v4().to_string(),
        ordinal,
        item_kind: requested.item_kind.clone(),
        object_id: requested.object_id.clone(),
        parent_object_id: parent,
        content_sha256: sha256,
        object_revision: revision,
        snapshot_hash,
        created_at,
    })
}

fn validate_task(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    draft: &AiTaskSpecDraft,
) -> Result<(), AiError> {
    validate_task_before_policy(draft)?;
    (policy.task_shape)(draft)?;
    validate_retry_shape(draft)
}

pub(crate) fn validate_task_before_policy(draft: &AiTaskSpecDraft) -> Result<(), AiError> {
    // Portraits freeze bounded reusable tag definitions and templates in the goal.
    let goal_limit = if draft.task_kind == "paper_profile_classification" {
        64_000
    } else {
        2_000
    };
    if !matches!(
        draft.task_kind.as_str(),
        "provider_diagnostic"
            | "selection_explanation"
            | "paper_profile_classification"
            | "paper_overview"
            | "paper_concept_map"
            | "paper_suggestions"
            | "reference_extraction"
            | "matrix_synthesis"
    ) || draft.user_goal.len() > goal_limit
        || draft.pipeline_version.trim().is_empty()
        || draft.prompt_version.trim().is_empty()
        || draft.output_schema_version < 1
        || !(1..=1_000_000).contains(&draft.max_input_tokens)
        || !(1..=100_000).contains(&draft.max_output_tokens)
        || !(1_000..=300_000).contains(&draft.timeout_ms)
    {
        return Err(AiError::InvalidInput("AI 任务参数无效".to_owned()));
    }
    if draft.task_kind != "paper_overview" && draft.overview_profile_selection_revision.is_some()
        || draft
            .overview_profile_selection_revision
            .is_some_and(|revision| revision < 1)
    {
        return Err(AiError::InvalidInput("AI 阅读画像版本参数无效".to_owned()));
    }
    if draft.paper_template_ref.is_some()
        && (draft.task_kind != "paper_overview"
            || draft.overview_profile_selection_revision.is_some()
            || draft.base_candidate_id.is_some())
    {
        return Err(AiError::InvalidInput(
            "本文模板不可与旧框架或重试引用混用".into(),
        ));
    }
    Ok(())
}

fn validate_task_scope(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    task_kind: &str,
    items: &[AiScopeItem],
) -> Result<(), AiError> {
    if matches!(
        task_kind,
        "paper_suggestions" | "reference_extraction" | "matrix_synthesis"
    ) {
        return (policy.task_scope)(task_kind, items);
    }
    validate_fixed_task_scope(task_kind, items)
}

pub(crate) fn validate_fixed_task_scope(
    task_kind: &str,
    items: &[AiScopeItem],
) -> Result<(), AiError> {
    let is_valid = match task_kind {
        "provider_diagnostic" => items.is_empty(),
        "selection_explanation" => items.len() == 1 && items[0].item_kind == "anchor",
        "paper_overview" | "paper_concept_map" => {
            (1..=8).contains(&items.len())
                && items.iter().all(|item| item.item_kind == "attachment")
        }
        "paper_profile_classification" => {
            (1..=8).contains(&items.len())
                && items.iter().all(|item| {
                    item.item_kind == "attachment"
                        && item.parent_object_id.is_some()
                        && item.parent_object_id == items[0].parent_object_id
                })
        }
        _ => false,
    };
    if is_valid {
        Ok(())
    } else {
        Err(AiError::InvalidInput("任务类型与显式范围不匹配".to_owned()))
    }
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, AiError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| AiError::InvalidInput("无法规范化 AI 任务范围".to_owned()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl ResolvedTaskScope {
    pub(crate) fn into_preview(
        self,
        draft: &AiTaskSpecDraft,
        scope_hash: String,
        overview_profile: Option<crate::ai::model::AiOverviewProfileContext>,
        matrix_synthesis_context: Option<crate::ai::model::AiMatrixSynthesisContext>,
        evidence_preview: Vec<crate::ai::model::AiEvidencePreview>,
    ) -> AiTaskPreview {
        AiTaskPreview {
            task_kind: draft.task_kind.clone(),
            provider_id: self.provider_id,
            model_id: self.model_id,
            reasoning_effort: self.reasoning_effort,
            is_cloud: self.is_cloud,
            scope_hash,
            input_item_count: self.items.len() as i64,
            scope_items: self.items,
            requires_cloud_consent: self.is_cloud,
            evidence_preview,
            overview_profile,
            suggestion_candidate_kind: draft.suggestion_candidate_kind.clone(),
            matrix_synthesis_context,
        }
    }
}

pub(crate) fn scope_hash(
    draft: &AiTaskSpecDraft,
    items: &[AiScopeItem],
    retry: Option<&crate::ai::model::AiOverviewRetryContext>,
    overview_profile: Option<&crate::ai::model::AiOverviewProfileContext>,
    matrix_synthesis_context: Option<&crate::ai::model::AiMatrixSynthesisContext>,
) -> Result<String, AiError> {
    hash_json(&TaskScopeFingerprint {
        schema_version: 4,
        task_kind: &draft.task_kind,
        item_snapshot_hashes: items
            .iter()
            .map(|item| item.snapshot_hash.as_str())
            .collect(),
        overview_retry: retry,
        overview_profile,
        suggestion_candidate_kind: &draft.suggestion_candidate_kind,
        matrix_synthesis_context,
    })
}
