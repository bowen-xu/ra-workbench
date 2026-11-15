//! Checkpoints use the existing owned file task and bibliography recovery guard.
mod decisions;
mod list;
mod resources;
mod save;
use super::super::{data_failure, failure};
use super::{FileOperation, FileResult};
use crate::{
    runtime_plugins::native::{NativeHostError, NativeResult},
    runtime_plugins::{RuntimePluginCatalog, native::NativePreviewService},
    storage::AppStore,
};
pub(crate) use decisions::record_matches;
use research_workbench_native_sdk::{
    data::DataRows,
    writing_checkpoint::{
        CheckpointDecision, CheckpointInput, CheckpointPlan, CheckpointRecord, CheckpointWrite,
    },
};
pub(crate) use resources::Resources;
use std::sync::Arc;

pub(crate) enum CheckpointCall {
    List,
    Save { write: Box<CheckpointWrite> },
}
#[derive(Clone)]
struct Origin {
    instance_id: String,
    request_id: String,
}
impl Origin {
    fn attach(&self, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
        if error.instance_id.is_none() && error.request_id.is_none() {
            error.instance_id = Some(self.instance_id.clone());
            error.request_id = Some(self.request_id.clone());
        }
        error
    }
}
#[derive(Clone)]
struct Ready<T> {
    value: T,
    origin: Origin,
}
struct Candidate<T> {
    rows: DataRows,
    result: NativeResult<Ready<T>>,
}
fn origin<T>(result: &NativeResult<Ready<T>>) -> Option<Origin> {
    match result {
        Ok(ready) => Some(ready.origin.clone()),
        Err(error) => error
            .instance_id
            .as_ref()
            .zip(error.request_id.as_ref())
            .map(|(instance_id, request_id)| Origin {
                instance_id: instance_id.clone(),
                request_id: request_id.clone(),
            }),
    }
}
impl FileOperation {
    fn checkpoint_mismatch(&self) -> Box<NativeHostError> {
        failure(
            "plugin_host_response_mismatch",
            "写作检查点决策与原请求或实际读取不匹配。",
            &self.key,
        )
    }
    pub(super) fn checkpoints(
        &self,
        call: &CheckpointCall,
        resources: &Resources,
        outcome: &mut crate::runtime_plugins::literature_catalog::save::SaveOutcome,
    ) -> NativeResult<FileResult> {
        match call {
            CheckpointCall::List => self.checkpoint_list(resources).map(FileResult::Checkpoints),
            CheckpointCall::Save { write } => self
                .checkpoint_save(resources, write, outcome)
                .map(|record| FileResult::Checkpoint(Box::new(record))),
        }
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_writing_document_checkpoint(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    write: CheckpointWrite,
) -> NativeResult<CheckpointRecord> {
    match super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        write.checkpoint.manuscript_id.clone(),
        super::FileRequest::Checkpoint {
            call: CheckpointCall::Save {
                write: Box::new(write),
            },
        },
    )
    .await?
    {
        FileResult::Checkpoint(record) => Ok(*record),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作检查点保存未返回原记录。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn list_runtime_writing_document_checkpoints(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Vec<CheckpointRecord>> {
    match super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        super::FileRequest::Checkpoint {
            call: CheckpointCall::List,
        },
    )
    .await?
    {
        FileResult::Checkpoints(records) => Ok(records),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作检查点读取未返回原记录。",
            &package_key,
        )),
    }
}
