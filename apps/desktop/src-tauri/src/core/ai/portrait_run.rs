//! Native portrait admission joins the original Core run manager before replying.
use super::{
    AiRunManager, JsonCredentialStore,
    error::AiError,
    generation_support::{AiRun, AiTaskSpecDraft},
};
use crate::storage::AppStore;

pub(super) async fn start(
    app: tauri::AppHandle,
    store: AppStore,
    credentials: JsonCredentialStore,
    manager: AiRunManager,
    draft: AiTaskSpecDraft,
) -> Result<AiRun, AiError> {
    // This accepted task retains source preparation, admission and dispatch
    // when its IPC waiter disappears. Execution then has the original manager.
    tokio::spawn(async move {
        let binding = store
            .native_note_trace()
            .cloned()
            .ok_or(AiError::CapabilityUnavailable(
                "literature_library.paper_generation_source",
            ))?;
        let preview = binding
            .service
            .preview_analysis_overview(
                store.clone(),
                credentials.clone(),
                &binding.package_key,
                draft.clone(),
            )
            .await?;
        if draft.confirmed_scope_hash.as_deref() != Some(preview.scope_hash.as_str()) {
            return Err(AiError::InvalidInput(
                "任务范围已变化，请重新确认外发预览".into(),
            ));
        }
        if preview.requires_cloud_consent
            && draft.cloud_consent_version.as_deref() != Some("ai-cloud-consent-v1")
        {
            return Err(AiError::InvalidInput(
                "启动云端任务前必须确认本次外发范围".into(),
            ));
        }
        let paper = preview
            .scope_items
            .first()
            .and_then(|scope| scope.parent_object_id.as_deref())
            .ok_or_else(|| AiError::InvalidInput("画像任务缺少所属文献".into()))?;
        let files = preview
            .scope_items
            .iter()
            .map(|scope| scope.object_id.clone())
            .collect::<Vec<_>>();
        super::paper_generation::prepare_sources(&store, paper, &files).await?;
        let run = binding
            .service
            .admit_analysis_overview(
                store.clone(),
                credentials.clone(),
                binding.package_key,
                draft,
                None,
            )
            .await?;
        super::commands::register_and_spawn(app, store, credentials, manager, run).await
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}
