//! Original archived-project deletion under the shared active-compile guard.
mod resources;
use super::super::{data_failure, decided_failure, failure, sql_error};
use super::{FileOperation, FileRequest, FileResult, operate_files};
use crate::{
    agent::writing_error::WritingError,
    compile_activity::CompileManager,
    data_interface::guarded_transaction,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{DecidedProject, NativePreviewService, NativeResult, SafeDatabaseError},
    },
    scoped_text_files::legacy,
    storage::AppStore,
};
use research_workbench_native_sdk::{
    data::DataRows,
    writing_project::{Decision, Input},
};
pub(super) use resources::DeletionResources;
use rusqlite::{Connection, DropBehavior, Transaction};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn delete_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    manager: tauri::State<'_, CompileManager>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<()> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::DeleteProject {
            manager: manager.inner().clone(),
        },
    )
    .await?
    {
        FileResult::Deleted => Ok(()),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "稿件删除结果种类不匹配。",
            &package_key,
        )),
    }
}

impl FileOperation {
    pub(super) fn delete_project(
        &self,
        manager: &CompileManager,
        resources: &DeletionResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        // Keep the original map guard through both Native waits and final I/O.
        // The nested result preserves the unchanged manager error contract.
        manager
            .with_idle_manuscript(&self.manuscript_id, || {
                Ok(self.prepare_deletion(resources, outcome))
            })
            .map_err(|error| {
                let code = if matches!(error, WritingError::CompileAlreadyRunning) {
                    "compile_already_running"
                } else {
                    "compile_process_error"
                };
                failure(code, error.to_string(), &self.key)
            })?
    }

    fn prepare_deletion(
        &self,
        resources: &DeletionResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let id = &self.manuscript_id;
        // Same identifier preflight as the original command, before any DB read.
        legacy::manuscript_source_directory(self.store.manuscript_directory(), id)
            .map_err(|error| self.file_error(error))?;
        let manuscript = {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources.projects.get(&db, id).map_err(data_failure)?
        };
        let prepared = self.decide(Input::PrepareDeletion {
            manuscript_id: id.clone(),
            manuscript: manuscript.clone(),
        })?;
        self.require_deletion(&prepared, false)?;
        let active = (|| {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            resources.active(&db, id).map_err(data_failure)
        })()
        .map_err(|mut error| {
            error.instance_id = Some(prepared.instance_id);
            error.request_id = Some(prepared.request_id);
            error
        })?;
        let verified = self.decide(Input::VerifyDeletion {
            manuscript_id: id.clone(),
            active_compile: active.clone(),
        })?;
        self.require_deletion(&verified, true)?;
        let mut guard = self.store.connection().map_err(|error| {
            decided_failure(
                "storage_error",
                error.to_string(),
                &self.key,
                &verified.instance_id,
                &verified.request_id,
            )
        })?;
        guarded_transaction::run(&mut guard, |db| {
            self.delete_in(db, resources, &manuscript, &active, outcome)
        })
        .map_err(|mut error| {
            error.instance_id = Some(verified.instance_id);
            error.request_id = Some(verified.request_id);
            error
        })?;
        Ok(FileResult::Deleted)
    }

    fn require_deletion(&self, decided: &DecidedProject, verified: bool) -> NativeResult<()> {
        let valid = match &decided.output.decision {
            Decision::DeletionNeedsCompileCheck { manuscript_id } if !verified => {
                manuscript_id == &self.manuscript_id
            }
            Decision::Deletion { manuscript_id } if verified => {
                manuscript_id == &self.manuscript_id
            }
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &self.key,
                    &decided.instance_id,
                    &decided.request_id,
                ));
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(decided_failure(
                "plugin_host_response_mismatch",
                "稿件删除决策阶段或身份不匹配。",
                &self.key,
                &decided.instance_id,
                &decided.request_id,
            ))
        }
    }

    fn delete_in(
        &self,
        db: &Connection,
        resources: &DeletionResources,
        manuscript: &DataRows,
        active: &DataRows,
        outcome: &mut SaveOutcome,
    ) -> (NativeResult<()>, bool) {
        let mut tx = match db.unchecked_transaction() {
            Ok(tx) => tx,
            Err(cause) => {
                return (
                    Err(sql_error(&self.key, "delete_begin", &cause)),
                    !db.is_autocommit(),
                );
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let result = self.remove_project_in(&tx, resources, manuscript, active, outcome);
        let error = match result {
            Ok(()) => match tx.commit() {
                Ok(()) if db.is_autocommit() => {
                    *outcome = SaveOutcome::Committed {
                        item_id: self.manuscript_id.clone(),
                    };
                    return (Ok(()), false);
                }
                Ok(()) => {
                    return (
                        Err(failure(
                            "storage_error",
                            "writing_project_delete_transaction_unsettled",
                            &self.key,
                        )),
                        true,
                    );
                }
                Err(cause) => {
                    let error = sql_error(&self.key, "delete_commit", &cause);
                    if db.is_autocommit() {
                        return (Err(error), true);
                    }
                    error
                }
            },
            Err(error) => return self.delete_rollback(db, error, tx.rollback()),
        };
        self.delete_rollback(db, error, db.execute_batch("ROLLBACK"))
    }

    fn delete_rollback(
        &self,
        db: &Connection,
        mut error: Box<crate::runtime_plugins::native::NativeHostError>,
        rollback: rusqlite::Result<()>,
    ) -> (NativeResult<()>, bool) {
        let cause = match rollback {
            Ok(()) if db.is_autocommit() => return (Err(error), false),
            Err(cause) => SafeDatabaseError::sql("delete_rollback", &cause),
            Ok(()) => SafeDatabaseError::invariant(
                "writing_project_delete_transaction_unsettled",
                "delete_rollback",
            ),
        };
        error.save_rollback_error = Some(Box::new(cause));
        (Err(error), true)
    }

    fn remove_project_in(
        &self,
        tx: &Transaction<'_>,
        resources: &DeletionResources,
        manuscript: &DataRows,
        active: &DataRows,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let id = &self.manuscript_id;
        self.resources
            .projects
            .recheck(tx, id, manuscript)
            .map_err(data_failure)?;
        resources.recheck(tx, id, active)?;
        let source = self.store.manuscript_directory().join(id);
        let cache = self.store.writing_cache_directory().join(id);
        // Preflight both roots before removing either, as in the original path.
        legacy::validate_directory(self.store.manuscript_directory(), &source)
            .map_err(|error| self.file_error(error))?;
        legacy::validate_directory(self.store.writing_cache_directory(), &cache)
            .map_err(|error| self.file_error(error))?;
        *outcome = SaveOutcome::Unknown;
        legacy::remove_directory(&source).map_err(|error| self.file_error(error))?;
        legacy::remove_directory(&cache).map_err(|error| self.file_error(error))?;
        resources.delete(tx, id).map_err(data_failure)
    }
}
