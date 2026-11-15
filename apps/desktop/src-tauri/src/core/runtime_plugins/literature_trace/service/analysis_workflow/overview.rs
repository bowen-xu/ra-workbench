//! Original overview preview and model/scope reads, without a static policy source.
use super::*;
use crate::{
    ai::{
        JsonCredentialStore,
        error::AiError,
        generation_support::{AiTaskPreview, AiTaskSpecDraft},
    },
    storage::AppStore,
};
mod admission;
mod capture;
mod conversation;
mod pages;
use capture::{CaptureError, OverviewPolicy};

impl LiteratureTraceService {
    pub(crate) async fn build_analysis_overview_task(
        &self,
        key: &str,
        input: research_workbench_native_sdk::analysis_workflow::overview_task::OverviewTaskInputV1,
    ) -> Result<AiTaskSpecDraft, AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(key, WorkflowInputV1::OverviewTask { input })
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::OverviewTask { draft_cbor_base64 } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "结构化阅读任务结果种类不匹配。",
                &origin,
            ));
        };
        tokio::task::spawn_blocking(move || decode_payload(&draft_cbor_base64))
            .await
            .map_err(|_| {
                template::identified(
                    "plugin_host_source_worker_unsettled",
                    "结构化阅读任务解码未确认完成。",
                    &origin,
                )
            })?
            .map_err(|_| {
                template::identified(
                    "plugin_host_response_mismatch",
                    "结构化阅读任务结果格式不匹配。",
                    &origin,
                )
            })
    }
    pub(crate) async fn preview_analysis_overview(
        &self,
        store: AppStore,
        credentials: JsonCredentialStore,
        key: &str,
        draft: AiTaskSpecDraft,
    ) -> Result<AiTaskPreview, AiError> {
        self.prepare_overview_policy(store, credentials, key, draft)
            .await
            .map(|(_, preview)| preview)
    }
    async fn prepare_overview_policy(
        &self,
        store: AppStore,
        credentials: JsonCredentialStore,
        key: &str,
        draft: AiTaskSpecDraft,
    ) -> Result<(OverviewPolicy, AiTaskPreview), AiError> {
        let mut policy = OverviewPolicy::default();
        // Each wait adds the next exact observation in the original read order.
        loop {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let secrets = credentials.clone();
            let task = draft.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = match source.connection() {
                    Ok(db) => policy.capture(&db, &secrets, &task),
                    Err(error) => Err(CaptureError::Original(Box::new(error.into()))),
                };
                (policy, result)
            })
            .await
            .map_err(|_| {
                template::technical(
                    Self::error(
                        "plugin_host_source_worker_unsettled",
                        "结构化阅读运行预览未确认完成。",
                    )
                    .package(key),
                )
            })?;
            policy = next;
            match result {
                Ok(preview) => return Ok((policy, preview)),
                Err(CaptureError::Original(error)) => return Err(*error),
                Err(CaptureError::Need(input)) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(key, *input.clone())
                        .await
                        .map_err(template::technical)?;
                    policy.record(*input, decision, origin);
                }
                Err(CaptureError::MatrixResources) => {
                    let catalog = Arc::clone(&self.catalog);
                    let selected = key.to_owned();
                    policy.matrix_sql = Some(tokio::task::spawn_blocking(move || {
                        let package = package::accept_analysis_workflow(&catalog, &selected).map_err(template::technical)?;
                        crate::runtime_plugins::literature_matrix::context::MatrixContextSql::load(&catalog, &package)
                            .map_err(|cause| template::technical(Self::error(cause.code, cause.message).package(&selected)))
                    }).await.map_err(|_| policy.incomplete_error(key))??);
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_sources;
