//! Original source export over the selected inspection and shared Core writer.
use super::*;
use crate::{agent::writing_error::WritingError, writing_transfer_files};
use research_workbench_native_sdk::{writing_project::ManuscriptExportResult, writing_transfer};

impl FileOperation {
    pub(super) fn export_source(
        &self,
        destination: &str,
        rename_on_conflict: bool,
    ) -> NativeResult<ManuscriptExportResult> {
        let mut native_error = None;
        let mut origin = None;
        let result = writing_transfer_files::export_numbered(
            destination,
            rename_on_conflict,
            |destination| {
                let destination =
                    writing_transfer_files::validated_destination(destination, "zip")?;
                writing_transfer_files::reject_managed_destination(&self.store, &destination)?;
                let source = crate::scoped_text_files::legacy::manuscript_source_directory(
                    self.store.manuscript_directory(),
                    &self.manuscript_id,
                )?;
                let inspection = (|| {
                    let rows = {
                        let db = self.store.connection().map_err(|cause| {
                            failure("storage_error", cause.to_string(), &self.key)
                        })?;
                        self.resources
                            .projects
                            .get(&db, &self.manuscript_id)
                            .map_err(data_failure)?
                    };
                    project_reply(
                        &self.key,
                        self.decide(Input::GetProject {
                            manuscript_id: self.manuscript_id.clone(),
                            rows,
                        })?,
                    )?;
                    let decided = self.decide(Input::Transfer {
                        input: Box::new(writing_transfer::Input::InspectExport {
                            directory: source.to_string_lossy().into(),
                        }),
                    })?;
                    origin = Some((decided.instance_id.clone(), decided.request_id.clone()));
                    let decision = match decided.output.decision {
                        Decision::Transfer { decision } => *decision,
                        _ => {
                            return Err(failure(
                                "plugin_host_response_mismatch",
                                "源码检查结果种类不匹配。",
                                &self.key,
                            ));
                        }
                    };
                    match decision {
                        writing_transfer::Decision::Files { files } => Ok(files),
                        writing_transfer::Decision::Rejected { code, message } => {
                            Err(failure(code.as_str(), message, &self.key))
                        }
                        _ => Err(failure(
                            "plugin_host_response_mismatch",
                            "源码检查结果种类不匹配。",
                            &self.key,
                        )),
                    }
                })();
                let files = match inspection {
                    Ok(files) => files,
                    Err(error) => {
                        native_error = Some(error);
                        // Stop the original numbering loop. Return the intact Native
                        // error below instead of converting its code or request identity.
                        return Err(WritingError::Process("源码检查未完成".into()));
                    }
                };
                writing_transfer_files::export_source(&source, &destination, &files)
            },
        );
        result.map_err(|cause| {
            let mut error =
                native_error.unwrap_or_else(|| failure(cause.code(), cause.to_string(), &self.key));
            if error.request_id.is_none()
                && let Some((instance, request)) = origin
            {
                error.instance_id = Some(instance);
                error.request_id = Some(request);
            }
            error
        })
    }
}

#[tauri::command]
pub(crate) async fn export_runtime_manuscript_source(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    destination_path: String,
    rename_on_conflict: Option<bool>,
) -> NativeResult<ManuscriptExportResult> {
    match operate_files(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key.clone(),
        manuscript_id,
        FileRequest::ExportSource {
            destination: destination_path.trim().into(),
            rename_on_conflict: rename_on_conflict.unwrap_or(false),
        },
    )
    .await?
    {
        FileResult::Exported(result) => Ok(result),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "源码导出结果种类不匹配。",
            &package_key,
        )),
    }
}
