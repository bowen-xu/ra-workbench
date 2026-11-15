use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    candidate_repository::get_candidate,
    candidate_schema::OVERVIEW_FIELDS,
    error::AiError,
    model::{
        AiOverviewProfileContext, AiOverviewRetryContext, AiReadingProfileRef, AiScopeItem,
        AiTaskSpecDraft,
    },
};

pub(crate) fn resolve_overview_profile(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    draft: &AiTaskSpecDraft,
    scope_item: &AiScopeItem,
    retry: Option<&AiOverviewRetryContext>,
) -> Result<Option<AiOverviewProfileContext>, AiError> {
    if let Some(retry) = retry {
        let candidate = get_candidate(connection, &retry.candidate_id)?;
        return load_overview_profile(connection, &candidate.run_id);
    }
    if let Some(reference) = &draft.paper_template_ref {
        let paper_id = scope_item
            .parent_object_id
            .as_deref()
            .ok_or_else(|| AiError::InvalidInput("模板提取附件缺少所属文献".into()))?;
        return super::paper_template::resolve(policy, connection, paper_id, reference).map(Some);
    }
    let Some(selection_revision) = draft.overview_profile_selection_revision else {
        return Ok(None);
    };
    let paper_id = scope_item
        .parent_object_id
        .as_deref()
        .ok_or_else(|| AiError::InvalidInput("概要附件缺少所属文献".to_owned()))?;
    resolve_selection(connection, paper_id, selection_revision).map(Some)
}

pub(crate) fn resolve_selection(
    connection: &Connection,
    paper_id: &str,
    selection_revision: i64,
) -> Result<AiOverviewProfileContext, AiError> {
    let refs_json = connection
        .query_row(
            "select profile_refs_json from paper_reading_profile_selections
             where paper_id = ?1 and revision = ?2",
            params![paper_id, selection_revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            AiError::InvalidInput("阅读画像未确认或版本已变化，请重新预览".to_owned())
        })?;
    let profile_refs: Vec<AiReadingProfileRef> = serde_json::from_str(&refs_json)
        .map_err(|_| AiError::InvalidInput("阅读画像选择记录已损坏".to_owned()))?;
    let field_kinds = resolve_fields(connection, &profile_refs)?;
    Ok(AiOverviewProfileContext {
        paper_template: None,
        paper_id: paper_id.to_owned(),
        selection_revision,
        profile_refs,
        field_kinds,
    })
}

pub(crate) fn insert_overview_profile(
    transaction: &Transaction<'_>,
    run_id: &str,
    context: &AiOverviewProfileContext,
) -> Result<(), AiError> {
    if context.paper_template.is_some() {
        return super::paper_template::insert_run(transaction, run_id, context);
    }
    transaction.execute(
        "insert into ai_overview_profile_runs
         (run_id, paper_id, selection_revision, profile_refs_json, field_kinds_json, created_at)
         values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            run_id,
            context.paper_id,
            context.selection_revision,
            serde_json::to_string(&context.profile_refs)
                .map_err(|_| AiError::InvalidInput("无法保存 AI 阅读画像".to_owned()))?,
            serde_json::to_string(&context.field_kinds)
                .map_err(|_| AiError::InvalidInput("无法保存 AI 阅读画像字段".to_owned()))?,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_overview_profile(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AiOverviewProfileContext>, AiError> {
    if let Some(context) = super::paper_template::load_run(connection, run_id)? {
        return Ok(Some(context));
    }
    let stored = connection
        .query_row(
            "select paper_id, selection_revision, profile_refs_json, field_kinds_json
             from ai_overview_profile_runs where run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(|(paper_id, selection_revision, refs_json, fields_json)| {
            let profile_refs = serde_json::from_str::<Vec<AiReadingProfileRef>>(&refs_json)
                .map_err(|_| AiError::InvalidInput("AI 阅读画像记录已损坏".to_owned()))?;
            let field_kinds = serde_json::from_str::<Vec<String>>(&fields_json)
                .map_err(|_| AiError::InvalidInput("AI 阅读画像字段记录已损坏".to_owned()))?;
            validate_context(&profile_refs, &field_kinds)?;
            Ok(AiOverviewProfileContext {
                paper_template: None,
                paper_id,
                selection_revision,
                profile_refs,
                field_kinds,
            })
        })
        .transpose()
}

fn resolve_fields(
    connection: &Connection,
    profile_refs: &[AiReadingProfileRef],
) -> Result<Vec<String>, AiError> {
    let mut requested = HashSet::new();
    for reference in profile_refs {
        let fields_json = connection
            .query_row(
                "select field_kinds_json from reading_profiles
                 where id = ?1 and version = ?2 and review_status = 'approved'",
                params![reference.profile_id, reference.profile_version],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| AiError::InvalidInput("阅读画像版本不存在或未经审核".to_owned()))?;
        let fields: Vec<String> = serde_json::from_str(&fields_json)
            .map_err(|_| AiError::InvalidInput("阅读画像字段记录已损坏".to_owned()))?;
        if fields.is_empty()
            || fields
                .iter()
                .any(|field| !OVERVIEW_FIELDS.contains(&field.as_str()))
        {
            return Err(AiError::InvalidInput("阅读画像包含未知字段".to_owned()));
        }
        requested.extend(fields);
    }
    let ordered = OVERVIEW_FIELDS
        .iter()
        .filter(|field| requested.contains(**field))
        .map(|field| (*field).to_owned())
        .collect::<Vec<_>>();
    validate_context(profile_refs, &ordered)?;
    Ok(ordered)
}

fn validate_context(
    profile_refs: &[AiReadingProfileRef],
    field_kinds: &[String],
) -> Result<(), AiError> {
    let unique_refs = profile_refs
        .iter()
        .map(|reference| (reference.profile_id.as_str(), reference.profile_version))
        .collect::<HashSet<_>>();
    let unique_fields = field_kinds.iter().collect::<HashSet<_>>();
    if profile_refs.is_empty()
        || profile_refs.len() > 3
        || unique_refs.len() != profile_refs.len()
        || field_kinds.is_empty()
        || field_kinds.len() > OVERVIEW_FIELDS.len()
        || unique_fields.len() != field_kinds.len()
        || field_kinds
            .iter()
            .any(|field| !OVERVIEW_FIELDS.contains(&field.as_str()))
    {
        return Err(AiError::InvalidInput("AI 阅读画像上下文无效".to_owned()));
    }
    Ok(())
}
