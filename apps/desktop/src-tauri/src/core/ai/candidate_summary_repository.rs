use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{
    candidate_repository::get_candidate,
    error::AiError,
    model::{
        AiCandidate, AiCandidateSummaryDiscardDraft, AiCandidateSummarySelectionDraft,
        AiCandidateSummaryVersion, AiCandidateSummaryVersionDraft,
    },
};

const SUMMARY_LIMIT: usize = 1_200;

pub(crate) fn list_summary_versions(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiCandidateSummaryVersion>, AiError> {
    let mut statement = connection.prepare(
        "select id, candidate_id, candidate_revision, version, summary_text,
                source_kind, is_selected, created_at, discarded_at
         from ai_candidate_summary_versions
         where candidate_id = ?1
         order by candidate_revision, version",
    )?;
    Ok(statement
        .query_map([candidate_id], |row| {
            Ok(AiCandidateSummaryVersion {
                id: row.get(0)?,
                candidate_id: row.get(1)?,
                candidate_revision: row.get(2)?,
                version: row.get(3)?,
                summary: row.get(4)?,
                source_kind: row.get(5)?,
                is_selected: row.get::<_, i64>(6)? == 1,
                created_at: row.get(7)?,
                discarded_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn create_summary_version(
    connection: &mut Connection,
    draft: AiCandidateSummaryVersionDraft,
) -> Result<AiCandidate, AiError> {
    validate_idempotency_key(&draft.idempotency_key)?;
    let candidate = get_candidate(connection, &draft.candidate_id)?;
    validate_summary_target(&candidate, draft.candidate_revision)?;
    if let Some(existing_id) = connection
        .query_row(
            "select id from ai_candidate_summary_versions
             where candidate_id = ?1 and idempotency_key = ?2",
            params![draft.candidate_id, draft.idempotency_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        select_summary_version(
            connection,
            AiCandidateSummarySelectionDraft {
                candidate_id: draft.candidate_id,
                candidate_revision: draft.candidate_revision,
                summary_version_id: existing_id,
            },
        )?;
        return get_candidate(connection, &candidate.id);
    }
    let revision = candidate
        .revisions
        .iter()
        .find(|item| item.revision == draft.candidate_revision)
        .ok_or_else(|| AiError::InvalidInput("找不到要摘要的候选版本".to_owned()))?;
    let summary = generate_selection_summary(&revision.payload)?;
    let next_version = connection.query_row(
        "select coalesce(max(version), 0) + 1
         from ai_candidate_summary_versions
         where candidate_id = ?1 and candidate_revision = ?2",
        params![draft.candidate_id, draft.candidate_revision],
        |row| row.get::<_, i64>(0),
    )?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "update ai_candidate_summary_versions
         set is_selected = 0
         where candidate_id = ?1 and candidate_revision = ?2 and discarded_at is null",
        params![draft.candidate_id, draft.candidate_revision],
    )?;
    transaction.execute(
        "insert into ai_candidate_summary_versions
         (id, candidate_id, candidate_revision, version, summary_text, source_kind,
          is_selected, idempotency_key, created_at)
         values (?1, ?2, ?3, ?4, ?5, 'local_extractive', 1, ?6,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            Uuid::new_v4().to_string(),
            draft.candidate_id,
            draft.candidate_revision,
            next_version,
            summary,
            draft.idempotency_key,
        ],
    )?;
    transaction.commit()?;
    get_candidate(connection, &candidate.id)
}

pub(crate) fn select_summary_version(
    connection: &mut Connection,
    draft: AiCandidateSummarySelectionDraft,
) -> Result<AiCandidate, AiError> {
    let candidate = get_candidate(connection, &draft.candidate_id)?;
    validate_summary_target(&candidate, draft.candidate_revision)?;
    ensure_available_summary(
        connection,
        &draft.candidate_id,
        draft.candidate_revision,
        &draft.summary_version_id,
    )?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "update ai_candidate_summary_versions
         set is_selected = 0
         where candidate_id = ?1 and candidate_revision = ?2 and discarded_at is null",
        params![draft.candidate_id, draft.candidate_revision],
    )?;
    transaction.execute(
        "update ai_candidate_summary_versions
         set is_selected = 1
         where id = ?1",
        [draft.summary_version_id],
    )?;
    transaction.commit()?;
    get_candidate(connection, &candidate.id)
}

pub(crate) fn discard_other_summary_versions(
    connection: &mut Connection,
    draft: AiCandidateSummaryDiscardDraft,
) -> Result<AiCandidate, AiError> {
    let candidate = get_candidate(connection, &draft.candidate_id)?;
    validate_summary_target(&candidate, draft.candidate_revision)?;
    ensure_available_summary(
        connection,
        &draft.candidate_id,
        draft.candidate_revision,
        &draft.keep_summary_version_id,
    )?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "update ai_candidate_summary_versions
         set discarded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), is_selected = 0
         where candidate_id = ?1 and candidate_revision = ?2 and id <> ?3
               and discarded_at is null",
        params![
            draft.candidate_id,
            draft.candidate_revision,
            draft.keep_summary_version_id,
        ],
    )?;
    transaction.execute(
        "update ai_candidate_summary_versions
         set is_selected = 1
         where id = ?1",
        [draft.keep_summary_version_id],
    )?;
    transaction.commit()?;
    get_candidate(connection, &candidate.id)
}

fn validate_summary_target(
    candidate: &AiCandidate,
    candidate_revision: i64,
) -> Result<(), AiError> {
    if candidate.candidate_kind != "selection_explanation_v1" {
        return Err(AiError::InvalidInput(
            "当前仅支持为 AI 选区解释生成摘要".to_owned(),
        ));
    }
    if candidate.current_revision != candidate_revision {
        return Err(AiError::InvalidInput(
            "候选已产生新版本，请重新载入后再生成摘要".to_owned(),
        ));
    }
    if !candidate
        .revisions
        .iter()
        .any(|item| item.revision == candidate_revision)
    {
        return Err(AiError::InvalidInput("找不到要摘要的候选版本".to_owned()));
    }
    Ok(())
}

fn ensure_available_summary(
    connection: &Connection,
    candidate_id: &str,
    candidate_revision: i64,
    summary_version_id: &str,
) -> Result<(), AiError> {
    let exists = connection
        .query_row(
            "select 1 from ai_candidate_summary_versions
             where id = ?1 and candidate_id = ?2 and candidate_revision = ?3
                   and discarded_at is null",
            params![summary_version_id, candidate_id, candidate_revision],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(AiError::InvalidInput("找不到可用的摘要版本".to_owned()))
    }
}

fn generate_selection_summary(payload: &serde_json::Value) -> Result<String, AiError> {
    if payload.get("kind").and_then(serde_json::Value::as_str) != Some("selection_explanation_v1") {
        return Err(AiError::InvalidInput(
            "当前仅支持为 AI 选区解释生成摘要".to_owned(),
        ));
    }
    let statements = payload
        .get("statements")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AiError::InvalidInput("候选缺少可摘要陈述".to_owned()))?;
    let mut parts = statements
        .iter()
        .filter_map(|statement| statement.get("text").and_then(serde_json::Value::as_str))
        .map(normalize_space)
        .filter(|text| !text.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(AiError::InvalidInput("候选缺少可摘要陈述".to_owned()));
    }
    if statements.len() > parts.len() {
        parts.push(format!("共 {} 条陈述", statements.len()));
    }
    let mut summary = parts.join("；");
    if let Some(first_uncertainty) = payload
        .get("uncertainties")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .and_then(serde_json::Value::as_str)
        .map(normalize_space)
        .filter(|text| !text.is_empty())
    {
        summary.push_str("。不确定性：");
        summary.push_str(&first_uncertainty);
    }
    Ok(truncate_chars(&summary, SUMMARY_LIMIT))
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut truncated = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn validate_idempotency_key(value: &str) -> Result<(), AiError> {
    let length = value.trim().chars().count();
    if (1..=200).contains(&length) {
        Ok(())
    } else {
        Err(AiError::InvalidInput("摘要生成请求标识无效".to_owned()))
    }
}
