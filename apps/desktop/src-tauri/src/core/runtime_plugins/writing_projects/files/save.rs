//! Same file owner, original physical save, then the original index/catalog tx.
use super::super::{data_failure, decided_failure, failure, sql_error};
use super::{
    FileOperation, FileRequest, FileResult, WriteMode, catalog::CatalogResources, operate_files,
};
use crate::runtime_plugins::native::NativePreviewService;
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{DecidedProject, NativeHostError, NativeResult, SafeDatabaseError},
    },
    scoped_text_files::TextFileError,
    storage::AppStore,
};
use research_workbench_native_sdk::{
    data::DataRows,
    writing_file::{FileSavePlan, WritingNotePreparation},
    writing_project::{Decision, Input, ManuscriptFileContent, SaveFileRequest},
};
use rusqlite::{Connection, DropBehavior};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn create_runtime_manuscript_file(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
    content: String,
) -> NativeResult<ManuscriptFileContent> {
    let request = SaveFileRequest {
        manuscript_id: manuscript_id.clone(),
        relative_path,
        content,
        expected_sha256: None,
    };
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Create { request },
    )
    .await?
    {
        FileResult::Read(file) => Ok(file),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作文件创建结果种类不匹配。",
            &package_key,
        )),
    }
}

impl FileOperation {
    pub(super) fn write(
        &self,
        request: &SaveFileRequest,
        mode: WriteMode,
        catalog: &CatalogResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = self.decide(Input::PrepareFileSave {
            request: request.clone(),
            note_title: crate::storage::schema_compat::writing_note_title(
                request.relative_path.trim(),
            ),
        })?;
        let preparation = match output.decision {
            Decision::FileSavePrepared { preparation } if preparation.request == *request => {
                *preparation
            }
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &self.key,
                    &instance_id,
                    &request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "文件保存准备未保持原请求。",
                    &self.key,
                    &instance_id,
                    &request_id,
                ));
            }
        };
        let captured = {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources
                .projects
                .get(&db, request.manuscript_id.trim())
                .map_err(data_failure)?
        };
        let note_analysis = match &preparation.note {
            WritingNotePreparation::Document { note } => Some(Box::new(
                crate::notes::document_analysis::analyze_with_external_assets(
                    note.document.clone(),
                ),
            )),
            _ => None,
        };
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = self.decide(Input::FinishFileSave {
            preparation: Box::new(preparation.clone()),
            manuscript: captured.clone(),
            note_analysis,
        })?;
        let plan = match output.decision {
            Decision::FileSave { plan } if plan.preparation == preparation => *plan,
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &self.key,
                    &instance_id,
                    &request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "文件保存计划未保持原请求。",
                    &self.key,
                    &instance_id,
                    &request_id,
                ));
            }
        };
        self.persist_file(&plan, mode, &captured, catalog, outcome)
            .map_err(|mut error| {
                error.instance_id = Some(instance_id);
                error.request_id = Some(request_id);
                error
            })
    }

    fn persist_file(
        &self,
        plan: &FileSavePlan,
        mode: WriteMode,
        captured: &DataRows,
        catalog: &CatalogResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let request = &plan.preparation.request;
        let id = request.manuscript_id.trim();
        let path = request.relative_path.trim();
        let mut db = self
            .store
            .connection()
            .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
        self.resources
            .projects
            .recheck(&db, id, captured)
            .map_err(data_failure)?;
        // Retain the original connection owner through file replacement and SQL.
        // No Native wait occurs under this owner or the following transaction.
        let files = self.store.manuscript_files(id);
        let hash = match mode {
            WriteMode::Save => {
                files.save(path, &request.content, request.expected_sha256.as_deref())
            }
            WriteMode::Create => files.create(path, &request.content),
        }
        .map_err(|cause| {
            if matches!(cause, TextFileError::Io(_)) {
                *outcome = SaveOutcome::Unknown;
            }
            self.file_error(cause)
        })?;
        *outcome = SaveOutcome::Unknown;
        let unsettled = self.index_saved(&db, plan, catalog, &hash);
        if !db.is_autocommit() {
            db.require_recovery();
        }
        unsettled?;
        *outcome = SaveOutcome::Committed { item_id: id.into() };
        Ok(FileResult::Read(ManuscriptFileContent {
            relative_path: request.relative_path.clone(),
            content: request.content.clone(),
            sha256: hash,
        }))
    }

    fn index_saved(
        &self,
        db: &Connection,
        plan: &FileSavePlan,
        catalog: &CatalogResources,
        hash: &str,
    ) -> NativeResult<()> {
        let mut tx = db
            .unchecked_transaction()
            .map_err(|cause| sql_error(&self.key, "begin", &cause))?;
        tx.set_drop_behavior(DropBehavior::Ignore);
        let request = &plan.preparation.request;
        let result = self
            .resources
            .index(
                &tx,
                request.manuscript_id.trim(),
                request.relative_path.trim(),
                &plan.preparation.file_kind,
                request.content.len(),
                hash,
            )
            .map_err(data_failure)
            .and_then(|()| catalog.record(&tx, plan, hash));
        let error = match result {
            Ok(()) => match tx.commit() {
                Ok(()) if db.is_autocommit() => return Ok(()),
                Ok(()) => {
                    return Err(failure(
                        "storage_error",
                        "writing_file_index_transaction_unsettled",
                        &self.key,
                    ));
                }
                Err(cause) => sql_error(&self.key, "commit", &cause),
            },
            Err(error) => {
                let rollback = tx.rollback();
                return Err(self.rollback_error(db, error, rollback));
            }
        };
        if db.is_autocommit() {
            return Err(error);
        }
        Err(self.rollback_error(db, error, db.execute_batch("ROLLBACK")))
    }

    fn rollback_error(
        &self,
        db: &Connection,
        mut error: Box<NativeHostError>,
        rollback: rusqlite::Result<()>,
    ) -> Box<NativeHostError> {
        let diagnostic = match rollback {
            Ok(()) if db.is_autocommit() => return error,
            Err(cause) => SafeDatabaseError::sql("rollback", &cause),
            Ok(()) => {
                SafeDatabaseError::invariant("writing_file_index_transaction_unsettled", "rollback")
            }
        };
        error.save_rollback_error = Some(Box::new(diagnostic));
        error
    }
}
