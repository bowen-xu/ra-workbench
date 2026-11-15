//! Original rename/delete critical sections over Native-prepared file policy.
mod resources;
use super::super::{data_failure, decided_failure, failure, project_reply};
use super::{FileOperation, FileRequest, FileResult, operate_files};
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{NativePreviewService, NativeResult},
    },
    scoped_text_files::TextFileError,
    storage::AppStore,
};
use research_workbench_native_sdk::{
    data::DataRows,
    writing_file::{FileChangeAction, FileChangePlan, FileChangeRequest},
    writing_project::{Decision, Input, Manuscript, MetadataDecision},
};
pub(crate) use resources::ChangeResources;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn rename_runtime_manuscript_file(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    from_path: String,
    to_path: String,
) -> NativeResult<Manuscript> {
    let request = FileChangeRequest::Rename {
        manuscript_id: manuscript_id.clone(),
        from_path,
        to_path,
    };
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Change { request },
    )
    .await?
    {
        FileResult::Project(manuscript) => Ok(manuscript),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "文件改名结果种类不匹配。",
            &package_key,
        )),
    }
}

#[tauri::command]
pub(crate) async fn delete_runtime_manuscript_file(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<()> {
    let request = FileChangeRequest::Delete {
        manuscript_id: manuscript_id.clone(),
        relative_path,
    };
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Change { request },
    )
    .await?
    {
        FileResult::Deleted => Ok(()),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "文件删除结果种类不匹配。",
            &package_key,
        )),
    }
}

enum ChangeExecution {
    Project(Manuscript),
    Readback(DataRows),
    Deleted,
}

impl FileOperation {
    pub(super) fn change(
        &self,
        request: &FileChangeRequest,
        resources: &ChangeResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let prepared = self.decide(Input::PrepareFileChange {
            request: request.clone(),
        })?;
        match prepared.output.decision {
            Decision::FileChangePrepared { request: original } if original == *request => {}
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &self.key,
                    &prepared.instance_id,
                    &prepared.request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "文件变更准备未保持原请求。",
                    &self.key,
                    &prepared.instance_id,
                    &prepared.request_id,
                ));
            }
        }
        let captured = {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources
                .projects
                .get(&db, request.manuscript_id().trim())
                .map_err(data_failure)?
        };
        let decided = self.decide(Input::PlanFileChange {
            request: request.clone(),
            manuscript: captured.clone(),
        })?;
        let plan = match decided.output.decision {
            Decision::FileChange { plan }
                if plan.request == *request
                    && plan.manuscript.id == request.manuscript_id().trim() =>
            {
                *plan
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
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "文件变更计划未保持原请求。",
                    &self.key,
                    &decided.instance_id,
                    &decided.request_id,
                ));
            }
        };
        let result = self
            .persist_change(&plan, &captured, resources, outcome)
            .map_err(|mut error| {
                error.instance_id = Some(decided.instance_id);
                error.request_id = Some(decided.request_id);
                error
            })?;
        match result {
            ChangeExecution::Project(manuscript) => Ok(FileResult::Project(manuscript)),
            ChangeExecution::Deleted => Ok(FileResult::Deleted),
            ChangeExecution::Readback(rows) => {
                let result = self.decide(Input::GetProject {
                    manuscript_id: plan.manuscript.id,
                    rows,
                })?;
                project_reply(&self.key, result).map(FileResult::Project)
            }
        }
    }

    fn persist_change(
        &self,
        plan: &FileChangePlan,
        captured: &DataRows,
        resources: &ChangeResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ChangeExecution> {
        let id = &plan.manuscript.id;
        let mut db = self
            .store
            .connection()
            .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
        self.resources
            .projects
            .recheck(&db, id, captured)
            .map_err(data_failure)?;
        let files = self.store.manuscript_files(id);
        let result = (|| {
            let result = match (&plan.request, &plan.action) {
                (
                    FileChangeRequest::Rename {
                        from_path, to_path, ..
                    },
                    FileChangeAction::Rename {
                        touch_destination,
                        on_unindexed,
                        root_metadata,
                    },
                ) => {
                    files
                        .rename(from_path.trim(), to_path.trim(), *touch_destination)
                        .map_err(|cause| {
                            if matches!(cause, TextFileError::Io(_)) {
                                *outcome = SaveOutcome::Unknown;
                            }
                            self.file_error(cause)
                        })?;
                    *outcome = SaveOutcome::Unknown;
                    let count = resources
                        .rename(&db, id, from_path.trim(), to_path.trim())
                        .map_err(data_failure)?;
                    if count == 0 {
                        return Err(failure(
                            on_unindexed.code(),
                            on_unindexed.to_string(),
                            &self.key,
                        ));
                    }
                    match root_metadata {
                        None => ChangeExecution::Project(plan.manuscript.clone()),
                        Some(MetadataDecision::Rejected { reason }) => {
                            return Err(failure(reason.code(), reason.to_string(), &self.key));
                        }
                        Some(MetadataDecision::Ready { metadata }) => {
                            resources
                                .metadata
                                .write(&db, id, metadata)
                                .map_err(data_failure)?;
                            ChangeExecution::Readback(
                                self.resources.projects.get(&db, id).map_err(data_failure)?,
                            )
                        }
                    }
                }
                (FileChangeRequest::Delete { relative_path, .. }, FileChangeAction::Delete) => {
                    files.delete(relative_path.trim()).map_err(|cause| {
                        if matches!(cause, TextFileError::Io(_)) {
                            *outcome = SaveOutcome::Unknown;
                        }
                        self.file_error(cause)
                    })?;
                    *outcome = SaveOutcome::Unknown;
                    resources
                        .delete(&db, id, relative_path.trim())
                        .map_err(data_failure)?;
                    ChangeExecution::Deleted
                }
                _ => {
                    return Err(failure(
                        "plugin_host_response_mismatch",
                        "文件变更动作与原请求不匹配。",
                        &self.key,
                    ));
                }
            };
            *outcome = SaveOutcome::Committed {
                item_id: id.clone(),
            };
            Ok(result)
        })();
        if !db.is_autocommit() {
            db.require_recovery();
        }
        result
    }
}
