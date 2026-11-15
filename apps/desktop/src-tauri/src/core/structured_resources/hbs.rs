//! Shared full-document commit for typed HBS clients. Existing validation,
//! provenance, verification and revision storage retain ownership.
use super::{content, repository, validation};
use rusqlite::{Transaction, params};
use serde_json::{Value, json};
fn storage_error(error: super::Error) -> String {
    let code = match &error {
        super::Error::NotFound => "MODEL_NOT_FOUND",
        super::Error::Conflict => "MODEL_VERSION_CONFLICT",
        super::Error::Invalid(_) | super::Error::Evidence(_) => "MODEL_INVALID_DOCUMENT",
        super::Error::LegacyImportRequired => "MODEL_LEGACY_IMPORT_REQUIRED",
        super::Error::LegacyImportConflict => "MODEL_LEGACY_IMPORT_CONFLICT",
        super::Error::Database(_) => "MODEL_STORAGE_ERROR",
    };
    format!("{code}：{error}")
}

use crate::ai::structured_resource_validation::{Failure, FailureCause, Flow, StructuredPolicy};
use crate::hbs_storage::boundary::HbsSources;
fn storage_failure(error: super::Error) -> Failure<String> {
    storage_failure_at(error, "read")
}
fn storage_failure_at(error: super::Error, phase: &'static str) -> Failure<String> {
    let cause = match &error {
        super::Error::Database(cause) => FailureCause::SourceDatabase(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql(phase, cause),
        )),
        _ => FailureCause::Original,
    };
    Failure {
        error: storage_error(error),
        cause,
    }
}
/// Preserve the original non-template source gate, while templates use only policy.
pub(crate) fn prepare_validation_sources<S: HbsSources>(
    sources: &mut S,
    db: &rusqlite::Connection,
    id: &str,
) -> Result<(), Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    if !sources.eager_validation_sources() {
        let resource =
            repository::get_header(db, id).map_err(|error| Flow::Fail(storage_failure(error)))?;
        if resource.kind == "readingTemplate" {
            return Ok(());
        }
    }
    sources.require().map(|_| ())
}
pub(crate) fn validate_preserved_with<S: HbsSources>(
    sources: &mut S,
    db: &rusqlite::Connection,
    id: &str,
    value: &Value,
) -> Result<(), Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    if value.is_null() {
        return Ok(());
    }
    let resource = repository::get(db, id).map_err(|error| Flow::Fail(storage_failure(error)))?;
    validation::validate_review_lazy_with(sources, db, &resource, &value.to_string())
        .map(|_| ())
        .map_err(|error| {
            error.map_error(|errors| {
                format!(
                    "MODEL_RECOVERY_INVALID：{}",
                    errors
                        .iter()
                        .map(|error| format!("{} {} {}", error.code, error.path, error.message))
                        .collect::<Vec<_>>()
                        .join("；")
                )
            })
        })
}

pub(crate) fn read(db: &rusqlite::Connection, id: &str) -> Result<Value, String> {
    read_failure(db, id).map_err(|failure| failure.error)
}
pub(crate) fn read_failure(db: &rusqlite::Connection, id: &str) -> Result<Value, Failure<String>> {
    let resource = repository::get(db, id).map_err(storage_failure)?;
    Ok(
        json!({"version":resource.revision,"saved":resource.content,"baseline":resource.review_baseline}),
    )
}
pub(crate) fn commit_with<S: HbsSources>(
    sources: &mut S,
    tx: &Transaction<'_>,
    id: &str,
    expected: i64,
    saved: Value,
    baseline: Value,
) -> Result<Value, Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    let mut resource =
        repository::get(tx, id).map_err(|error| Flow::Fail(storage_failure(error)))?;
    if resource.revision != expected {
        return Err(Flow::original(
            "MODEL_VERSION_CONFLICT：结构化资源已更新".into(),
        ));
    }
    if let Some(paper) = resource.paper_id.as_deref() {
        super::require_legacy_imported(tx, paper)
            .map_err(|error| Flow::Fail(storage_failure(error)))?;
    }
    let draft_pending: bool = tx
        .query_row(
            "select exists(select 1 from structured_resource_drafts where resource_id=?1)",
            [id],
            |row| row.get(0),
        )
        .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "read", &error)))?;
    if draft_pending {
        return Err(Flow::original(
            "MODEL_DRAFT_PENDING：资源仍有修复草稿，请先在原草稿流程处理；正文、基线与草稿均已保留"
                .into(),
        ));
    }
    if saved.to_string().len() > 8 * 1024 * 1024 || baseline.to_string().len() > 8 * 1024 * 1024 {
        return Err(Flow::original(
            "MODEL_REQUEST_TOO_LARGE：结构化正文或基线超过容量限制".into(),
        ));
    }
    let checked = if saved.is_null() {
        None
    } else {
        Some(
            validation::validate_review_lazy_with(sources, tx, &resource, &saved.to_string())
                .map_err(|error| {
                    error.map_error(|errors| {
                        format!(
                            "MODEL_INVALID_DOCUMENT：{}",
                            errors
                                .iter()
                                .map(|e| format!("{} {} {}", e.code, e.path, e.message))
                                .collect::<Vec<_>>()
                                .join("；")
                        )
                    })
                })?,
        )
    };
    if !baseline.is_null() {
        validation::validate_review_lazy_with(sources, tx, &resource, &baseline.to_string())
            .map_err(|error| {
                error.map_error(|errors| {
                    format!(
                        "MODEL_INVALID_BASELINE：{}",
                        errors
                            .iter()
                            .map(|e| format!("{} {} {}", e.code, e.path, e.message))
                            .collect::<Vec<_>>()
                            .join("；")
                    )
                })
            })?;
    }
    let previous = content::fields(resource.content.as_ref());
    let next = content::fields(checked.as_ref());
    let changed = content::pending(checked.as_ref(), resource.content.as_ref());
    let previously_reviewed = resource.reviewed_field_ids.clone();
    resource
        .reviewed_field_ids
        .retain(|id| previous.get(id) == next.get(id));
    let protected: String = tx
        .query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "read", &error)))?;
    let protected: Vec<String> =
        serde_json::from_str(&protected).map_err(|error| Flow::original(error.to_string()))?;
    let mut protected = protected
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let before_baseline = content::fields(resource.review_baseline.as_ref());
    let after_baseline = content::fields(Some(&baseline));
    protected.retain(|id| before_baseline.get(id) == after_baseline.get(id));
    protected.extend(
        previously_reviewed
            .into_iter()
            .filter(|id| changed.contains(id)),
    );
    resource.content = checked;
    resource.review_baseline = if baseline.is_null() {
        None
    } else {
        Some(baseline)
    };
    resource.pending_field_ids =
        content::pending(resource.content.as_ref(), resource.review_baseline.as_ref());
    protected.retain(|id| resource.pending_field_ids.contains(id));
    resource.revision += 1;
    repository::persist_revision(
        tx,
        &resource,
        "human",
        None,
        &protected.into_iter().collect::<Vec<_>>(),
    )
    .map_err(|error| Flow::Fail(storage_failure_at(error, "write")))?;
    if let Some(name) = resource
        .content
        .as_ref()
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
    {
        tx.execute(
            "update structured_resources set title=?2 where id=?1",
            params![id, name],
        )
        .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "write", &error)))?;
    }
    read_failure(tx, id).map_err(Flow::Fail)
}
#[cfg(test)]
pub(crate) fn seed_with<S: HbsSources>(
    sources: &mut S,
    db: &mut rusqlite::Connection,
    paper: &str,
    value: Value,
) -> Result<(String, Value), Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle','HBS synthetic reading','now','now') on conflict(id) do nothing",params![paper]).map_err(|e| Flow::original(e.to_string()))?;
    repository::ensure_paper_resources(db, paper).map_err(|e| Flow::original(e.to_string()))?;
    let resource = repository::find_by_paper(db, paper, "structuredReading")
        .map_err(|e| Flow::original(e.to_string()))?;
    if resource.content.is_some() {
        return Ok((
            resource.id.clone(),
            read_failure(db, &resource.id).map_err(Flow::Fail)?,
        ));
    }
    let tx = db
        .transaction()
        .map_err(|e| Flow::original(e.to_string()))?;
    let result = commit_with(
        sources,
        &tx,
        &resource.id,
        resource.revision,
        value.clone(),
        value,
    )?;
    tx.commit().map_err(|e| Flow::original(e.to_string()))?;
    Ok((resource.id, result))
}
