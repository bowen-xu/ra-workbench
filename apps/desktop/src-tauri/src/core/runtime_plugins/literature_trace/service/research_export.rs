//! Core retains the original single export owner, guarded capture and physical publication.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    runtime_plugins::literature_export::{
        ResearchExportCommandError, ResearchExportError, ResearchExportManager,
        ResearchExportReport, ResearchExportSql, capture, writer,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    research_export::ResearchExportRejectionV1,
};

impl LiteratureTraceService {
    pub(crate) async fn research_export(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        manager: ResearchExportManager,
        target: String,
    ) -> NativeResult<ResearchExportReport> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move {
            owner.available()?;
            let _lifetime = owner.doi_operations.read().await;
            owner.available()?;
            let sql = owner.research_export_sql(&key).await?;
            manager
                .begin()
                .map_err(|error| failure(error, &key, None))?;
            let result = owner.export_owned(store, &key, &manager, target, sql).await;
            manager.finish();
            result
        })
        .await
        .map_err(|_| failure(ResearchExportError::TaskFailed, &selected, None))?
    }

    async fn export_owned(
        &self,
        store: AppStore,
        key: &str,
        manager: &ResearchExportManager,
        target: String,
        sql: ResearchExportSql,
    ) -> NativeResult<ResearchExportReport> {
        let source = store.clone();
        let worker = manager.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            let connection = source.connection()?;
            capture::capture(&connection, &sql, &worker)
        })
        .await
        .map_err(|_| failure(ResearchExportError::TaskFailed, key, None))?
        .map_err(|error| failure(error, key, None))?;
        // The original snapshot is complete here. Native never receives a connection or target path.
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::ResearchExport {
                    snapshot: Box::new(snapshot),
                },
            )
            .await?;
        let WorkflowDecisionV1::ResearchExport { result } = decision else {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "研究知识导出准备种类不匹配。",
            )
            .package(key));
        };
        let prepared = result.map_err(|reason| {
            let error = match reason {
                ResearchExportRejectionV1::InvalidSnapshot => ResearchExportError::InvalidSnapshot,
                ResearchExportRejectionV1::TooLarge => ResearchExportError::TooLarge,
                ResearchExportRejectionV1::Serialization { message } => {
                    ResearchExportError::Serialization(serde_json::Error::io(
                        std::io::Error::other(message),
                    ))
                }
            };
            failure(error, key, Some(&origin))
        })?;
        let worker = manager.clone();
        tokio::task::spawn_blocking(move || writer::write_export(&target, &prepared, &worker))
            .await
            .map_err(|_| failure(ResearchExportError::TaskFailed, key, Some(&origin)))?
            .map_err(|error| failure(error, key, Some(&origin)))
    }

    pub(crate) async fn cancel_research_export(
        &self,
        key: &str,
        manager: &ResearchExportManager,
    ) -> NativeResult<bool> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || package::accept_research_export(&catalog, &selected))
            .await
            .map_err(|_| Self::error("worker_unavailable", "导出资格读取线程不可用。"))??;
        Ok(manager.cancel())
    }

    async fn research_export_sql(&self, key: &str) -> NativeResult<ResearchExportSql> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            package::accept_analysis_workflow(&catalog, &selected)?;
            let package = package::accept_research_export(&catalog, &selected)?;
            ResearchExportSql::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "导出资源读取线程不可用。"))?
    }
}

fn failure(
    error: ResearchExportError,
    key: &str,
    origin: Option<&DecisionOrigin>,
) -> Box<NativeHostError> {
    let cause = error.to_string();
    let command: ResearchExportCommandError = error.into();
    let message = if command.code == "export_failed" {
        format!("{}；cause={cause}", command.message)
    } else {
        command.message
    };
    let mut failure = LiteratureTraceService::error(command.code, message).package(key);
    if let Some(origin) = origin {
        failure.instance_id = Some(origin.instance_id.clone());
        failure.request_id = Some(origin.native_request_id.clone());
    }
    failure
}
