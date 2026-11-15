//! Existing bounded preparation, with owned Native calls outside Core DB guards.
mod publication;
mod resources;
use super::super::{data_failure, failure};
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{
            DecidedProject, NativeHostError, NativePreviewService, NativeProfile, NativeResult,
            package,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue},
    writing_catalog::{
        CatalogDecision, CatalogInput, FileRead, ItemInput, ItemPlan, PendingFile, Preparation,
        PreparationScope,
    },
    writing_file::WritingNotePreparation,
    writing_project::{Decision, Input},
};
use resources::Resources;
use std::sync::Arc;

#[derive(Clone)]
struct Origin {
    instance_id: String,
    request_id: String,
}
impl Origin {
    fn attach(&self, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
        error.instance_id = Some(self.instance_id.clone());
        error.request_id = Some(self.request_id.clone());
        error
    }
}

struct Operation {
    store: AppStore,
    service: Arc<NativePreviewService>,
    resources: Resources,
    key: String,
    runtime: tokio::runtime::Handle,
}

#[tauri::command]
pub(crate) async fn prepare_runtime_writing_note_catalog(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    retry_errors: bool,
    scope: Option<PreparationScope>,
) -> NativeResult<Preparation> {
    prepare_catalog(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        retry_errors,
        scope.unwrap_or_default(),
    )
    .await
}

pub(crate) async fn prepare_catalog(
    store: AppStore,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    retry_errors: bool,
    scope: PreparationScope,
) -> NativeResult<Preparation> {
    let operation_id = uuid::Uuid::new_v4().to_string();
    let selected = key.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let worker_service = Arc::clone(&service);
        let worker_key = selected.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &worker_key, NativeProfile::WritingProjects)?;
            let resources = Resources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &worker_key))?;
            let operation = Operation {
                store,
                service: worker_service,
                resources,
                key: worker_key,
                runtime,
            };
            let mut outcome = SaveOutcome::NotStarted;
            operation
                .run(retry_errors, scope, &mut outcome)
                .map_err(|mut error| {
                    error.save_outcome.get_or_insert(outcome);
                    error
                })
        })
        .await
        .map_err(|_| unknown(&selected))?
    })
    .await
    .map_err(|_| unknown(&key))?
    .map_err(|mut error| {
        error.operation_id = Some(operation_id);
        error.save_outcome.get_or_insert(SaveOutcome::NotStarted);
        error
    })
}

fn unknown(key: &str) -> Box<NativeHostError> {
    let mut error = failure(
        "index_failed",
        "writing_note_catalog_prepare_task_failed：目录准备任务未能结算，请核对原登记文件及目录状态。",
        key,
    );
    error.save_outcome = Some(SaveOutcome::Unknown);
    error
}

impl Operation {
    fn mismatch(&self, message: &str) -> Box<NativeHostError> {
        failure("plugin_host_response_mismatch", message, &self.key)
    }
    fn decide(&self, input: CatalogInput) -> NativeResult<(CatalogDecision, Origin)> {
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = self.runtime.block_on(self.service.project_decision(
            self.key.clone(),
            Input::CatalogPreparation {
                input: Box::new(input),
            },
        ))?;
        let origin = Origin {
            instance_id,
            request_id,
        };
        match output.decision {
            Decision::CatalogPreparation { decision } => Ok((*decision, origin)),
            _ => Err(origin.attach(self.mismatch("目录准备决策种类不匹配。"))),
        }
    }
    fn run(
        &self,
        retry_errors: bool,
        scope: PreparationScope,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Preparation> {
        let (decision, origin) = self.decide(CatalogInput::Batch {
            scope,
            retry_errors,
        })?;
        let CatalogDecision::Batch { statuses } = decision else {
            return Err(origin.attach(self.mismatch("目录准备批次阶段不匹配。")));
        };
        let rows = (|| {
            let mut guard = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            guarded_transaction::run(&mut guard, |db| {
                let result = (|| {
                    if retry_errors {
                        *outcome = SaveOutcome::Unknown;
                        self.resources.retry(db, &statuses).map_err(data_failure)?;
                    }
                    self.resources.pending(db, &statuses).map_err(data_failure)
                })();
                (result, !db.is_autocommit())
            })
        })()
        .map_err(|error| origin.attach(error))?;
        let files = self.pending(rows)?;
        let mut processed_count = 0;
        for file in files {
            let read = match self
                .store
                .manuscript_files(&file.manuscript_id)
                .read(&file.path)
            {
                Ok((content, hash)) => FileRead::Read { content, hash },
                Err(cause) => FileRead::Failed {
                    cause: crate::agent::writing_error::WritingError::from(cause).to_string(),
                },
            };
            let input = ItemInput {
                note_title: crate::storage::schema_compat::writing_note_title(&file.path),
                file: file.clone(),
                read,
            };
            // The original pending-hash check precedes consumption of all file
            // and projection results, including a technical Native failure.
            let candidate = self.item(input);
            if self.publish(&file, candidate, outcome)? {
                processed_count += 1;
            }
        }
        let rows = (|| {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources.counts(&db, &statuses).map_err(data_failure)
        })()
        .map_err(|error| origin.attach(error))?;
        let (decision, origin) = self.decide(CatalogInput::Finish {
            processed_count,
            rows: rows.clone(),
        })?;
        match decision {
            CatalogDecision::Finish { result }
                if result.processed_count == processed_count
                    && rows.rows
                        == [vec![
                            DataValue::Integer(result.pending_count),
                            DataValue::Integer(result.error_count),
                        ]] =>
            {
                Ok(result)
            }
            _ => Err(origin.attach(self.mismatch("目录准备计数与实际提交结果不匹配。"))),
        }
    }
    fn pending(&self, rows: DataRows) -> NativeResult<Vec<PendingFile>> {
        let (decision, origin) = self.decide(CatalogInput::Pending { rows: rows.clone() })?;
        let CatalogDecision::Pending { files } = decision else {
            return Err(origin.attach(self.mismatch("目录准备待处理阶段不匹配。")));
        };
        let decoded = DataRows {
            rows: files
                .iter()
                .map(|file| {
                    vec![
                        DataValue::Text(file.manuscript_id.clone()),
                        DataValue::Text(file.path.clone()),
                        DataValue::Text(file.hash.clone()),
                    ]
                })
                .collect(),
        };
        if decoded != rows {
            return Err(origin.attach(self.mismatch("目录准备文件与实际登记快照不匹配。")));
        }
        Ok(files)
    }
    fn item(&self, input: ItemInput) -> NativeResult<(ItemPlan, Origin)> {
        let (decision, origin) = self.decide(CatalogInput::PrepareItem {
            item: Box::new(input.clone()),
        })?;
        let CatalogDecision::PrepareItem { preparation } = decision else {
            return Err(origin.attach(self.mismatch("目录准备文档阶段不匹配。")));
        };
        if preparation.input != input {
            return Err(origin.attach(self.mismatch("目录准备文档与读取输入不匹配。")));
        }
        let analysis = match &preparation.note {
            Some(WritingNotePreparation::Document { note }) => Some(
                crate::notes::document_analysis::analyze_with_external_assets(
                    note.document.clone(),
                ),
            ),
            _ => None,
        };
        let (decision, origin) = self.decide(CatalogInput::FinishItem {
            preparation: preparation.clone(),
            analysis,
        })?;
        match decision {
            CatalogDecision::FinishItem { plan } if plan.preparation == *preparation => {
                Ok((*plan, origin))
            }
            _ => Err(origin.attach(self.mismatch("目录准备结果与原文档准备不匹配。"))),
        }
    }
}
