//! Original creation transaction and file cleanup; Native owns pure decisions.
mod resources;
use self::resources::CreationResources;
use super::{data_failure, decided_failure, failure, resources::ProjectReadResources};
use crate::{
    data_interface::{SharedDatabase, guarded_transaction, plugin_resources::PreparedDataRead},
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{
            NativeHostError, NativePreviewService, NativeProfile, NativeResult, SafeDatabaseError,
            package,
        },
    },
    scoped_text_files::legacy,
    storage::AppStore,
};
use research_workbench_native_sdk::{
    data::DataRows,
    writing_project::{
        CreationPlan, Decision, Input, Manuscript, ManuscriptDraft, MetadataDecision, ProjectScope,
        Rejection, STANDALONE_PROJECT_ID,
    },
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::sql_error;

#[tauri::command]
pub(crate) async fn create_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    draft: ManuscriptDraft,
) -> NativeResult<Manuscript> {
    create_project(
        store.shared_agent_connection(),
        store.manuscript_directory().to_owned(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        draft,
        uuid::Uuid::new_v4(),
    )
    .await
}

pub(super) async fn create_project(
    database: SharedDatabase,
    directory: PathBuf,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    draft: ManuscriptDraft,
    id: uuid::Uuid,
) -> NativeResult<Manuscript> {
    let operation_id = id.to_string();
    let selected = key.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let accepted_key = selected.clone();
        let (reads, writes) = tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &accepted_key, NativeProfile::WritingProjects)?;
            let reads = ProjectReadResources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            let writes = CreationResources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            Ok::<_, Box<NativeHostError>>((reads, writes))
        })
        .await
        .map_err(|_| {
            failure(
                "plugin_host_accept_failed",
                "写作工程资源核对任务未完成。",
                &selected,
            )
        })??;
        let requested_project = draft
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let captured = Arc::clone(&database);
        let capture_key = selected.clone();
        let project = requested_project.clone();
        let (writes, parent) = tokio::task::spawn_blocking(move || {
            let mut db = captured
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &capture_key))?;
            let result = guarded_transaction::run_mut(&mut db, |connection| {
                let mut commit_failed = false;
                let result: NativeResult<Option<PreparedDataRead>> = (|| {
                    let tx = connection
                        .transaction()
                        .map_err(|error| sql_error(&capture_key, "begin", &error))?;
                    let parent = project
                        .as_deref()
                        .map(|id| writes.capture_parent(&tx, id))
                        .transpose()
                        .map_err(data_failure)?;
                    tx.commit().map_err(|error| {
                        commit_failed = true;
                        sql_error(&capture_key, "read-commit", &error)
                    })?;
                    Ok(parent)
                })();
                let unknown = commit_failed || !connection.is_autocommit();
                (result, unknown)
            })?;
            Ok::<_, Box<NativeHostError>>((writes, result))
        })
        .await
        .map_err(|_| failure("storage_error", "写作工程归属读取任务未完成。", &selected))??;
        let prepared = service
            .project_decision(
                selected.clone(),
                Input::PrepareCreation {
                    manuscript_id: id.to_string(),
                    draft: draft.clone(),
                },
            )
            .await?;
        let plan = match prepared.output.decision {
            Decision::Creation { plan }
                if plan_matches(&plan, id, &draft, requested_project.as_deref()) =>
            {
                plan
            }
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &selected,
                    &prepared.instance_id,
                    &prepared.request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "写作工程创建准备未保持原身份与参数。",
                    &selected,
                    &prepared.instance_id,
                    &prepared.request_id,
                ));
            }
        };
        let verified = service
            .project_decision(
                selected.clone(),
                Input::VerifyCreation {
                    plan: plan.clone(),
                    project_exists: parent
                        .as_ref()
                        .map_or(DataRows { rows: vec![] }, |read| read.rows().clone()),
                },
            )
            .await?;
        match verified.output.decision {
            Decision::Creation { plan: returned } if returned == plan => {}
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &selected,
                    &verified.instance_id,
                    &verified.request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "写作工程归属核对未保持原计划。",
                    &selected,
                    &verified.instance_id,
                    &verified.request_id,
                ));
            }
        }
        let persist_key = selected.clone();
        let prepared_instance = verified.instance_id;
        let prepared_request = verified.request_id;
        let rows = tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &persist_key))?;
            guarded_transaction::run(&mut db, |connection| {
                persist(
                    connection,
                    &directory,
                    &reads,
                    &writes,
                    &plan,
                    parent.as_ref(),
                    &persist_key,
                )
            })
        })
        .await
        .map_err(|_| {
            let mut error = decided_failure(
                "storage_error",
                "写作工程创建结果未知；请按原稿件标识核对。",
                &selected,
                &prepared_instance,
                &prepared_request,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error
        })?
        .map_err(|mut error| {
            error.instance_id = Some(prepared_instance);
            error.request_id = Some(prepared_request);
            error
        })?;
        service
            .project_decision(
                selected.clone(),
                Input::GetProject {
                    manuscript_id: id.to_string(),
                    rows,
                },
            )
            .await
            .and_then(|decided| super::project_reply(&selected, decided))
            .map_err(|mut error| {
                error.save_outcome = Some(SaveOutcome::Committed {
                    item_id: id.to_string(),
                });
                error
            })
    })
    .await
    .map_err(|_| {
        let mut error = failure(
            "storage_error",
            "写作工程创建任务未能结算；请按原稿件标识核对。",
            &key,
        );
        error.save_outcome = Some(SaveOutcome::Unknown);
        error.operation_id = Some(operation_id.clone());
        error
    })?
    .map_err(|mut error| {
        error.operation_id = Some(operation_id);
        if error.save_outcome.is_none() {
            error.save_outcome = Some(SaveOutcome::NotStarted);
        }
        error
    })
}

fn plan_matches(
    plan: &CreationPlan,
    id: uuid::Uuid,
    draft: &ManuscriptDraft,
    project: Option<&str>,
) -> bool {
    let scope_matches = match (&plan.scope, project) {
        (ProjectScope::Existing { project_id }, Some(expected)) => project_id == expected,
        (ProjectScope::Standalone { project_id }, None) => project_id == STANDALONE_PROJECT_ID,
        _ => false,
    };
    let metadata_matches = match (&plan.metadata, &plan.initial_template) {
        (MetadataDecision::Ready { metadata }, Some(_)) => {
            metadata.title == draft.title.trim()
                && metadata.root_path == draft.root_path.as_deref().unwrap_or("main.tex")
        }
        (MetadataDecision::Rejected { .. }, None) => true,
        _ => false,
    };
    plan.manuscript_id == id.to_string() && scope_matches && metadata_matches
}

fn persist(
    connection: &Connection,
    directory: &Path,
    reads: &ProjectReadResources,
    writes: &CreationResources,
    plan: &CreationPlan,
    parent: Option<&PreparedDataRead>,
    key: &str,
) -> (NativeResult<DataRows>, bool) {
    let mut tx = match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(cause) => {
            return (
                Err(sql_error(key, "begin", &cause)),
                !connection.is_autocommit(),
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let mut template_written = false;
    let result = (|| {
        if let Some(parent) = parent {
            parent.recheck_in(&tx).map_err(data_failure)?;
        }
        if let ProjectScope::Standalone { project_id } = &plan.scope {
            writes.standalone(&tx, project_id).map_err(data_failure)?;
        }
        let metadata = match &plan.metadata {
            MetadataDecision::Ready { metadata } => metadata,
            MetadataDecision::Rejected { reason } => {
                return Err(failure(reason.code(), reason.to_string(), key));
            }
        };
        writes.insert(&tx, plan, metadata).map_err(data_failure)?;
        let rows = reads.get(&tx, &plan.manuscript_id).map_err(data_failure)?;
        if rows.rows.is_empty() {
            return Err(failure(
                Rejection::ManuscriptNotFound.code(),
                Rejection::ManuscriptNotFound.to_string(),
                key,
            ));
        }
        let initial = plan.initial_template.as_ref().ok_or_else(|| {
            failure(
                "plugin_host_response_mismatch",
                "写作创建计划缺少原模板。",
                key,
            )
        })?;
        let content = initial.content.as_str();
        let template = (|| {
            let root = legacy::manuscript_source_directory(directory, &plan.manuscript_id)?;
            let destination = root.join(legacy::validate_relative_path(&metadata.root_path)?);
            fs::create_dir_all(
                destination
                    .parent()
                    .ok_or(crate::scoped_text_files::TextFileError::InvalidPath)?,
            )?;
            legacy::atomic_write(&destination, content.as_bytes())
        })();
        if let Err(error) = template {
            let _ = writes.delete(&tx, &plan.manuscript_id);
            let code = error.code();
            return Err(failure(
                code,
                crate::agent::writing_error::WritingError::from(error).to_string(),
                key,
            ));
        }
        template_written = true;
        writes
            .index(
                &tx,
                &plan.manuscript_id,
                &metadata.root_path,
                &initial.kind,
                content.len(),
                &legacy::sha256(content.as_bytes()),
            )
            .map_err(data_failure)?;
        Ok(rows)
    })();
    let settled = match result {
        Ok(rows) => match tx.commit() {
            Ok(()) => return (Ok(rows), false),
            Err(cause) => {
                let error = sql_error(key, "commit", &cause);
                if connection.is_autocommit() {
                    (error, true, None)
                } else {
                    rollback_result(connection, error, connection.execute_batch("ROLLBACK"))
                }
            }
        },
        Err(error) => {
            let rollback = tx.rollback();
            rollback_result(connection, error, rollback)
        }
    };
    if template_written
        && let Ok(root) = legacy::manuscript_source_directory(directory, &plan.manuscript_id)
    {
        let _ = fs::remove_dir_all(root);
    }
    let (mut error, unknown, rollback_error) = settled;
    error.save_outcome = Some(if unknown {
        SaveOutcome::Unknown
    } else {
        SaveOutcome::RolledBack
    });
    error.save_rollback_error = rollback_error;
    (Err(error), unknown)
}

fn rollback_result(
    connection: &Connection,
    error: Box<NativeHostError>,
    result: rusqlite::Result<()>,
) -> (Box<NativeHostError>, bool, Option<Box<SafeDatabaseError>>) {
    match result {
        Ok(()) if connection.is_autocommit() => (error, false, None),
        Err(cause) => (
            error,
            true,
            Some(Box::new(SafeDatabaseError::sql("rollback", &cause))),
        ),
        Ok(()) => (
            error,
            true,
            Some(Box::new(SafeDatabaseError::invariant(
                "writing_creation_transaction_unsettled",
                "rollback",
            ))),
        ),
    }
}
